use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
};

use crate::target_configuration::{
    TargetConfigurationDiscovery, TargetInitializationError, TargetInitializationExpectation,
    discover_target_configuration_with_store, initialize_target_configuration_with_store,
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
    fn transaction_root(&self) -> &Path;
    fn find_in_path(&self) -> io::Result<Option<PathBuf>>;
    fn run(&self, executable: &Path, arguments: &[&str]) -> io::Result<CommandOutput>;
    fn inspect_target(&self, target: &Path) -> io::Result<TargetConfigurationDiscovery> {
        discover_target_configuration_with_store(target, self.transaction_root())
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
}
