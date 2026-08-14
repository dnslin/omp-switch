use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

use crate::target_configuration::{
    TargetConfigurationDiscovery, discover_target_configuration, initialize_target_configuration,
};

pub struct CommandOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

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
    fn find_in_path(&self) -> io::Result<Option<PathBuf>>;
    fn run(&self, executable: &Path, arguments: &[&str]) -> io::Result<CommandOutput>;
    fn inspect_target(&self, target: &Path) -> io::Result<TargetConfigurationDiscovery> {
        discover_target_configuration(target)
    }
    fn initialize_target(
        &self,
        target: &Path,
        expected_create_paths: &[String],
    ) -> io::Result<TargetConfigurationDiscovery> {
        initialize_target_configuration(target, expected_create_paths)
    }
}

pub struct SystemOmpEnvironment;

impl OmpEnvironment for SystemOmpEnvironment {
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
}
