use std::{
    io::{self, Read},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::os::unix::process::CommandExt;
#[cfg(windows)]
use std::os::windows::{io::AsRawHandle, process::CommandExt};

use tokio_util::sync::CancellationToken;
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
    },
};

use crate::target_configuration::{
    TargetConfigurationDiscovery, TargetInitializationError, TargetInitializationExpectation,
    discover_target_configuration_until, discover_target_configuration_with_store,
    initialize_target_configuration_with_store,
};

pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

#[derive(Debug)]
pub enum CommandRunError {
    Io(io::Error),
    Cancelled,
    TimedOut,
}
#[cfg(windows)]
struct WindowsProcessJob {
    handle: HANDLE,
}

#[cfg(windows)]
impl WindowsProcessJob {
    fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let configured = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if configured == 0 {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(io::Error::last_os_error());
        }

        Ok(Self { handle })
    }

    fn assign(&self, child: &std::process::Child) -> io::Result<()> {
        let assigned =
            unsafe { AssignProcessToJobObject(self.handle, child.as_raw_handle() as HANDLE) };
        if assigned == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn terminate(&self) {
        unsafe {
            let _ = TerminateJobObject(self.handle, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for WindowsProcessJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn resume_suspended_process(process_id: u32) -> io::Result<()> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return Err(io::Error::last_os_error());
    }

    let result = (|| {
        let mut entry = THREADENTRY32 {
            dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
            ..Default::default()
        };
        let mut has_entry = unsafe { Thread32First(snapshot, &mut entry) };
        if has_entry == 0 {
            return Err(io::Error::last_os_error());
        }

        while has_entry != 0 {
            if entry.th32OwnerProcessID == process_id {
                let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID) };
                if thread.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let resume_result = unsafe { ResumeThread(thread) };
                unsafe {
                    let _ = CloseHandle(thread);
                }
                if resume_result == u32::MAX {
                    return Err(io::Error::last_os_error());
                }
                return Ok(());
            }
            has_entry = unsafe { Thread32Next(snapshot, &mut entry) };
        }

        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "OMP suspended process thread unavailable",
        ))
    })();

    unsafe {
        let _ = CloseHandle(snapshot);
    }
    result
}
#[cfg(unix)]
type ProcessTree = ();

#[cfg(windows)]
type ProcessTree = WindowsProcessJob;

impl CommandOutput {
    #[cfg(test)]
    pub fn success(stdout: impl Into<String>) -> Self {
        Self {
            success: true,
            exit_code: Some(0),
            stdout: stdout.into(),
            stderr: String::new(),
        }
    }

    #[cfg(test)]
    pub fn failure(exit_code: i32, stderr: impl Into<String>) -> Self {
        Self {
            success: false,
            exit_code: Some(exit_code),
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

pub trait OmpEnvironment: Send + Sync {
    fn transaction_root(&self) -> &Path;
    fn find_in_path(&self) -> io::Result<Option<PathBuf>>;
    fn run(&self, executable: &Path, arguments: &[&str]) -> io::Result<CommandOutput>;
    fn run_with_deadline(
        &self,
        executable: &Path,
        arguments: &[&str],
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<CommandOutput, CommandRunError>;
    fn inspect_target(&self, target: &Path) -> io::Result<TargetConfigurationDiscovery> {
        discover_target_configuration_with_store(target, self.transaction_root())
    }

    fn inspect_target_with_deadline(
        &self,
        target: &Path,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<TargetConfigurationDiscovery, CommandRunError> {
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        let discovery = self.inspect_target(target).map_err(|error| {
            if cancellation.is_cancelled() {
                CommandRunError::Cancelled
            } else if Instant::now() >= deadline || error.kind() == io::ErrorKind::TimedOut {
                CommandRunError::TimedOut
            } else {
                CommandRunError::Io(error)
            }
        })?;
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(CommandRunError::TimedOut);
        }
        Ok(discovery)
    }
    fn initialize_target(
        &self,
        target: &Path,
        expectation: &TargetInitializationExpectation,
    ) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
        initialize_target_configuration_with_store(target, self.transaction_root(), expectation)
    }
}

pub struct SystemOmpEnvironment {
    transaction_root: PathBuf,
}

impl SystemOmpEnvironment {
    pub fn new(transaction_root: PathBuf) -> Self {
        Self { transaction_root }
    }
}

impl OmpEnvironment for SystemOmpEnvironment {
    fn transaction_root(&self) -> &Path {
        &self.transaction_root
    }

    fn find_in_path(&self) -> io::Result<Option<PathBuf>> {
        match which::which("omp") {
            Ok(path) => Ok(Some(path)),
            Err(which::Error::CannotFindBinaryPath) => Ok(None),
            Err(which::Error::CannotGetCurrentDirAndPathListEmpty) => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "system PATH is unavailable",
            )),
            Err(which::Error::CannotCanonicalize) => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "OMP path cannot be canonicalized",
            )),
        }
    }

    fn run(&self, executable: &Path, arguments: &[&str]) -> io::Result<CommandOutput> {
        let output = Command::new(executable).args(arguments).output()?;
        Ok(CommandOutput {
            success: output.status.success(),
            exit_code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }

    fn run_with_deadline(
        &self,
        executable: &Path,
        arguments: &[&str],
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<CommandOutput, CommandRunError> {
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        let mut command = Command::new(executable);
        command
            .args(arguments)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        command.process_group(0);
        #[cfg(windows)]
        command.creation_flags(windows_sys::Win32::System::Threading::CREATE_SUSPENDED);
        #[cfg(windows)]
        let process_job = WindowsProcessJob::new().map_err(CommandRunError::Io)?;

        let mut child = command.spawn().map_err(CommandRunError::Io)?;
        #[cfg(windows)]
        if let Err(error) = process_job.assign(&child) {
            process_job.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(CommandRunError::Io(error));
        }
        #[cfg(windows)]
        if let Err(error) = resume_suspended_process(child.id()) {
            process_job.terminate();
            let _ = child.kill();
            let _ = child.wait();
            return Err(CommandRunError::Io(error));
        }
        #[cfg(unix)]
        let process_tree = ();
        #[cfg(windows)]
        let process_tree = process_job;

        let process_id = child.id();
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                terminate_child(&mut child, process_id, &process_tree);

                return Err(CommandRunError::Io(io::Error::other(
                    "OMP stdout pipe unavailable",
                )));
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                terminate_child(&mut child, process_id, &process_tree);

                return Err(CommandRunError::Io(io::Error::other(
                    "OMP stderr pipe unavailable",
                )));
            }
        };
        let stdout_reader = thread::spawn(move || read_pipe(stdout));
        let stderr_reader = thread::spawn(move || read_pipe(stderr));

        let status = loop {
            if cancellation.is_cancelled() {
                terminate_child(&mut child, process_id, &process_tree);

                drop(stdout_reader);
                drop(stderr_reader);
                return Err(CommandRunError::Cancelled);
            }
            if Instant::now() >= deadline {
                terminate_child(&mut child, process_id, &process_tree);

                drop(stdout_reader);
                drop(stderr_reader);
                return Err(CommandRunError::TimedOut);
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => thread::sleep(Duration::from_millis(5)),
                Err(error) => {
                    terminate_child(&mut child, process_id, &process_tree);

                    drop(stdout_reader);
                    drop(stderr_reader);
                    return Err(CommandRunError::Io(error));
                }
            }
        };
        let stdout = match join_pipe_until(stdout_reader, cancellation, deadline) {
            Ok(stdout) => stdout,
            Err(error) => {
                terminate_child(&mut child, process_id, &process_tree);

                return Err(error);
            }
        };
        let stderr = match join_pipe_until(stderr_reader, cancellation, deadline) {
            Ok(stderr) => stderr,
            Err(error) => {
                terminate_child(&mut child, process_id, &process_tree);

                return Err(error);
            }
        };
        Ok(CommandOutput {
            success: status.success(),
            exit_code: status.code(),
            stdout: String::from_utf8_lossy(&stdout).into_owned(),
            stderr: String::from_utf8_lossy(&stderr).into_owned(),
        })
    }
    fn inspect_target_with_deadline(
        &self,
        target: &Path,
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<TargetConfigurationDiscovery, CommandRunError> {
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        let discovery = discover_target_configuration_until(target, cancellation, deadline)
            .map_err(|error| {
                if cancellation.is_cancelled() {
                    CommandRunError::Cancelled
                } else if Instant::now() >= deadline || error.kind() == io::ErrorKind::TimedOut {
                    CommandRunError::TimedOut
                } else {
                    CommandRunError::Io(error)
                }
            })?;
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(CommandRunError::TimedOut);
        }
        Ok(discovery)
    }
}

fn read_pipe(mut pipe: impl Read) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    pipe.read_to_end(&mut output)?;
    Ok(output)
}

fn join_pipe_until(
    reader: thread::JoinHandle<io::Result<Vec<u8>>>,
    cancellation: &CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, CommandRunError> {
    loop {
        if reader.is_finished() {
            return join_pipe(reader).map_err(CommandRunError::Io);
        }
        if cancellation.is_cancelled() {
            drop(reader);
            return Err(CommandRunError::Cancelled);
        }
        if Instant::now() >= deadline {
            drop(reader);
            return Err(CommandRunError::TimedOut);
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn join_pipe(reader: thread::JoinHandle<io::Result<Vec<u8>>>) -> io::Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| io::Error::other("OMP output reader failed"))?
}

fn terminate_child(child: &mut std::process::Child, _process_id: u32, process_tree: &ProcessTree) {
    #[cfg(unix)]
    {
        let _ = process_tree;
        terminate_process_group(_process_id);
    }
    #[cfg(windows)]
    process_tree.terminate();
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(unix)]
fn terminate_process_group(process_id: u32) {
    if process_id == 0 {
        return;
    }
    unsafe {
        let _ = libc::kill(-(process_id as libc::pid_t), libc::SIGKILL);
    }
}
