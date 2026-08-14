use std::{
    fs::{self, OpenOptions},
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigurationFileStatus {
    Normal,
    Missing,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetAccess {
    pub writable: bool,
    pub models_yml: ConfigurationFileStatus,
    pub config_yml: ConfigurationFileStatus,
}

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
    fn inspect_target(&self, target: &Path) -> io::Result<TargetAccess>;
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

    fn inspect_target(&self, target: &Path) -> io::Result<TargetAccess> {
        let access_root = if target.exists() {
            if !target.is_dir() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "target is not a directory",
                ));
            }
            target
        } else {
            target.parent().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidInput, "target has no parent")
            })?
        };
        Ok(TargetAccess {
            writable: probe_directory_write(access_root)?,
            models_yml: inspect_configuration_file(&target.join("models.yml"))?,
            config_yml: inspect_configuration_file(&target.join("config.yml"))?,
        })
    }
}

fn inspect_configuration_file(path: &Path) -> io::Result<ConfigurationFileStatus> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => Ok(ConfigurationFileStatus::Normal),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Ok(ConfigurationFileStatus::Missing)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
            Ok(ConfigurationFileStatus::ReadOnly)
        }
        Err(error) => Err(error),
    }
}

static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

fn probe_directory_write(directory: &Path) -> io::Result<bool> {
    let sequence = PROBE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let probe = directory.join(format!(
        ".omp-switch-access-{}-{sequence}",
        std::process::id()
    ));
    match OpenOptions::new().write(true).create_new(true).open(&probe) {
        Ok(file) => {
            drop(file);
            fs::remove_file(&probe)?;
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::{ConfigurationFileStatus, OmpEnvironment, SystemOmpEnvironment};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn real_target_probe_reports_files_and_cleans_up_probe() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), "models: []").unwrap();

        let access = SystemOmpEnvironment.inspect_target(&target).unwrap();

        assert!(access.writable);
        assert_eq!(access.models_yml, ConfigurationFileStatus::Normal);
        assert_eq!(access.config_yml, ConfigurationFileStatus::Missing);
        assert!(fs::read_dir(&target).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".omp-switch-access-")
        }));
    }

    #[cfg(unix)]
    #[test]
    fn real_target_probe_reports_read_only_file_and_directory() {
        use std::os::unix::fs::PermissionsExt;

        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let models = target.join("models.yml");
        fs::create_dir(&target).unwrap();
        fs::write(&models, "models: []").unwrap();
        fs::write(target.join("config.yml"), "providers: []").unwrap();
        fs::set_permissions(&models, fs::Permissions::from_mode(0o444)).unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o555)).unwrap();

        let access = SystemOmpEnvironment.inspect_target(&target).unwrap();

        fs::set_permissions(&target, fs::Permissions::from_mode(0o755)).unwrap();
        fs::set_permissions(&models, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!access.writable);
        assert_eq!(access.models_yml, ConfigurationFileStatus::ReadOnly);
        assert_eq!(access.config_yml, ConfigurationFileStatus::Normal);
        assert!(fs::read_dir(&target).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".omp-switch-access-")
        }));
    }

    #[test]
    fn real_target_probe_propagates_missing_parent_error() {
        let root = tempdir().unwrap();
        let target = root.path().join("missing-parent").join("agent");

        let error = SystemOmpEnvironment.inspect_target(&target).unwrap_err();

        assert_eq!(error.kind(), std::io::ErrorKind::NotFound);
    }
}
