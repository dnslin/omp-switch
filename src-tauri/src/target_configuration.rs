use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use cap_std::{
    ambient_authority,
    fs::{Dir, OpenOptions as CapOpenOptions},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(windows)]
use std::os::windows::{
    ffi::OsStrExt as _,
    io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle},
};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static INITIALIZATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetConfigurationStatus {
    Writable,
    ReadOnly,
    CreationRequired,
    MigrationRequired,
    ParseError,
    Unsafe,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConfigurationFileStatus {
    Normal,
    Missing,
    ReadOnly,
    AlternateOnly,
    CanonicalWithAlternate,
    LegacyJson,
    ParseError,
    Unsafe,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationFileDiscovery {
    pub canonical_path: String,
    pub resolved_path: Option<String>,
    pub status: ConfigurationFileStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigurationIssue {
    pub file_path: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TargetConfigurationDiscovery {
    pub path: String,
    pub resolved_path: Option<String>,
    pub status: TargetConfigurationStatus,
    pub writable: bool,
    pub models: ConfigurationFileDiscovery,
    pub config: ConfigurationFileDiscovery,
    pub recovery_notice: Option<String>,
    pub create_paths: Vec<String>,
    pub discovery_token: String,
    pub warnings: Vec<String>,
    pub issue: Option<ConfigurationIssue>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TargetInitializationExpectation {
    pub create_paths: Vec<String>,
    pub discovery_token: String,
}

#[derive(Debug, Error)]
#[error("{source}")]
pub struct TargetInitializationError {
    #[source]
    source: io::Error,
    recovery_incomplete: bool,
}

impl TargetInitializationError {
    fn new(source: io::Error, recovery_incomplete: bool) -> Self {
        Self {
            source,
            recovery_incomplete,
        }
    }

    pub fn recovery_incomplete(&self) -> bool {
        self.recovery_incomplete
    }

    pub fn kind(&self) -> io::ErrorKind {
        self.source.kind()
    }
}
#[derive(Debug, Error)]
#[error("Target configuration 在文件发布后失去已确认的文件系统身份：{source}")]
struct PostPublicationIdentityChange {
    #[source]
    source: io::Error,
}

fn post_publication_identity_error(source: io::Error) -> io::Error {
    io::Error::new(source.kind(), PostPublicationIdentityChange { source })
}

fn is_post_publication_identity_error(error: &io::Error) -> bool {
    error
        .get_ref()
        .and_then(|source| source.downcast_ref::<PostPublicationIdentityChange>())
        .is_some()
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "kebab-case")]
enum InitializationPhase {
    Prepared,
    Committed,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InitializationEntry {
    staging: PathBuf,
    destination: PathBuf,
    expected_final_hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct InitializationManifest {
    logical_target: PathBuf,
    phase: InitializationPhase,
    transaction_id: u64,
    process_id: u32,
    resolved_target: PathBuf,
    existing_ancestor: PathBuf,
    existing_ancestor_identity: String,
    resolved_target_identity: Option<String>,
    target_existed: bool,
    created_directories: Vec<PathBuf>,
    #[serde(default)]
    created_directory_identities: Vec<String>,
    #[serde(default)]
    directory_creation_in_progress: Option<usize>,
    entries: Vec<InitializationEntry>,
}
pub(crate) struct DiscoveryControl<'a> {
    cancellation: &'a CancellationToken,
    deadline: std::time::Instant,
}

impl DiscoveryControl<'_> {
    fn check(&self) -> io::Result<()> {
        if self.cancellation.is_cancelled() {
            return Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "model test cancelled",
            ));
        }
        if std::time::Instant::now() >= self.deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "model test timed out",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
pub fn discover_target_configuration(target: &Path) -> io::Result<TargetConfigurationDiscovery> {
    discover_target_configuration_with_store(target, &test_transaction_root(target)?)
}

pub fn discover_target_configuration_with_store(
    target: &Path,
    transaction_root: &Path,
) -> io::Result<TargetConfigurationDiscovery> {
    let recovery_notice = recover_interrupted_initialization(target, transaction_root)?;
    let mut discovery = discover_target_configuration_internal(target)?;
    discovery.recovery_notice = recovery_notice;
    Ok(discovery)
}
pub fn discover_target_configuration_until(
    target: &Path,
    cancellation: &CancellationToken,
    deadline: std::time::Instant,
) -> io::Result<TargetConfigurationDiscovery> {
    let control = DiscoveryControl {
        cancellation,
        deadline,
    };
    control.check()?;
    let discovery = discover_target_configuration_internal_with_control(target, Some(&control))?;
    control.check()?;
    Ok(discovery)
}

fn check_control(control: Option<&DiscoveryControl<'_>>) -> io::Result<()> {
    control.map_or(Ok(()), DiscoveryControl::check)
}

fn discover_target_configuration_internal(
    target: &Path,
) -> io::Result<TargetConfigurationDiscovery> {
    discover_target_configuration_internal_with_control(target, None)
}

fn discover_target_configuration_internal_with_control(
    target: &Path,
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<TargetConfigurationDiscovery> {
    check_control(control)?;
    let target_metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    check_control(control)?;
    let target_exists = target_metadata.is_some();
    let mut resolved_path = if target_exists {
        check_control(control)?;

        match target.canonicalize() {
            Ok(path) => Some(path.to_string_lossy().into_owned()),
            Err(error) if target_metadata.as_ref().is_some_and(is_link_or_reparse) => {
                return Ok(unsafe_discovery(
                    target,
                    format!("无法解析 Target configuration 链接或重解析点：{error}"),
                ));
            }
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    check_control(control)?;
    if target_exists {
        check_control(control)?;
    }
    if target_exists && !fs::metadata(target)?.is_dir() {
        return Ok(unsafe_discovery(
            target,
            "Target configuration 路径不是目录。".to_owned(),
        ));
    }
    let (models, models_issue) = discover_file(target, "models", &["models.json"], control)?;
    let (config, config_issue) =
        discover_file(target, "config", &["settings.json", "config.json"], control)?;
    let ancestor = ExistingAncestorWalk::new_with_control(target, control)?;
    if resolved_path.is_none() {
        resolved_path = Some(ancestor.expected_target.to_string_lossy().into_owned());
    }
    check_control(control)?;
    let directory_writable = probe_directory_write_with_control(&ancestor.existing_path, control)?;
    let status = combined_status(&models.status, &config.status, directory_writable);
    let writable = status == TargetConfigurationStatus::Writable;
    let create_paths = creation_paths(&ancestor, &models.status, &config.status);
    let mut warnings = Vec::new();
    for name in ["models", "config"] {
        if path_entry_exists_with_control(&target.join(format!("{name}.yml")), control)?
            && path_entry_exists_with_control(&target.join(format!("{name}.yaml")), control)?
        {
            warnings.push(format!(
                "检测到 {name}.yaml；OMP Switch 使用 {name}.yml，且 {name}.yaml 不会被修改。"
            ));
        }
    }
    let discovery_token = discovery_token(
        target,
        resolved_path.as_deref(),
        &ancestor,
        &status,
        &models,
        &config,
        &create_paths,
    )?;
    Ok(TargetConfigurationDiscovery {
        path: target.to_string_lossy().into_owned(),
        resolved_path,
        status,
        writable,
        models,
        config,
        create_paths,
        discovery_token,
        warnings,
        recovery_notice: None,
        issue: models_issue.or(config_issue),
    })
}

fn discover_file(
    target: &Path,
    stem: &str,
    legacy_names: &[&str],
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<(ConfigurationFileDiscovery, Option<ConfigurationIssue>)> {
    let canonical_path = target.join(format!("{stem}.yml"));
    let alternate_path = target.join(format!("{stem}.yaml"));
    let mut legacy_path = None;
    for name in legacy_names {
        let path = target.join(name);
        if path_entry_exists_with_control(&path, control)? {
            legacy_path = Some(path);
            break;
        }
    }
    let (mut status, selected_path) = if path_entry_exists_with_control(&canonical_path, control)? {
        let status = if path_entry_exists_with_control(&alternate_path, control)? {
            ConfigurationFileStatus::CanonicalWithAlternate
        } else {
            ConfigurationFileStatus::Normal
        };
        (status, Some(canonical_path.as_path()))
    } else if path_entry_exists_with_control(&alternate_path, control)? {
        (
            ConfigurationFileStatus::AlternateOnly,
            Some(alternate_path.as_path()),
        )
    } else if let Some(path) = legacy_path.as_deref() {
        (ConfigurationFileStatus::LegacyJson, Some(path))
    } else {
        (ConfigurationFileStatus::Missing, None)
    };
    let (resolved_path, resolution_issue) = if let Some(path) = selected_path {
        match path.canonicalize() {
            Ok(resolved) => {
                let metadata = fs::metadata(&resolved)?;
                if metadata.is_file() {
                    (Some(resolved.to_string_lossy().into_owned()), None)
                } else {
                    status = ConfigurationFileStatus::Unsafe;
                    (
                        Some(resolved.to_string_lossy().into_owned()),
                        Some(ConfigurationIssue {
                            file_path: path.to_string_lossy().into_owned(),
                            line: None,
                            column: None,
                            message: "配置路径解析后不是普通文件。".to_owned(),
                        }),
                    )
                }
            }
            Err(error) => {
                status = ConfigurationFileStatus::Unsafe;
                (
                    None,
                    Some(ConfigurationIssue {
                        file_path: path.to_string_lossy().into_owned(),
                        line: None,
                        column: None,
                        message: format!("无法解析配置文件链接或重解析点：{error}"),
                    }),
                )
            }
        }
    } else {
        (None, None)
    };
    let parse_issue = if resolution_issue.is_none() && status != ConfigurationFileStatus::LegacyJson
    {
        if let Some(path) = selected_path {
            match read_file_with_control(path, control) {
                Ok(contents) => match serde_yaml::from_slice::<serde_yaml::Value>(&contents) {
                    Ok(_) => None,
                    Err(error) => {
                        status = ConfigurationFileStatus::ParseError;
                        let location = error.location();
                        Some(ConfigurationIssue {
                            file_path: path.to_string_lossy().into_owned(),
                            line: location.as_ref().map(serde_yaml::Location::line),
                            column: location.as_ref().map(serde_yaml::Location::column),
                            message: error.to_string(),
                        })
                    }
                },
                Err(error) if is_read_only_error(&error) => {
                    status = ConfigurationFileStatus::ReadOnly;
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        }
    } else {
        None
    };
    if resolution_issue.is_none()
        && matches!(
            status,
            ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
        )
        && let Some(path) = selected_path
        && !file_is_writable_with_control(path, control)?
    {
        status = ConfigurationFileStatus::ReadOnly;
    }
    let issue = resolution_issue.or(parse_issue);
    Ok((
        ConfigurationFileDiscovery {
            canonical_path: canonical_path.to_string_lossy().into_owned(),
            resolved_path,
            status,
        },
        issue,
    ))
}

fn path_entry_exists(path: &Path) -> io::Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}
fn path_entry_exists_with_control(
    path: &Path,
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<bool> {
    check_control(control)?;
    let exists = path_entry_exists(path)?;
    check_control(control)?;
    Ok(exists)
}

fn read_file_with_control(
    path: &Path,
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<Vec<u8>> {
    check_control(control)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let mut file = options.open(path)?;
    check_control(control)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path is not a regular file",
        ));
    }
    let mut contents = Vec::new();
    let mut chunk = [0_u8; 8192];
    loop {
        check_control(control)?;
        match file.read(&mut chunk) {
            Ok(0) => return Ok(contents),
            Ok(read) => contents.extend_from_slice(&chunk[..read]),
            Err(error) => return Err(error),
        }
    }
}

fn file_is_writable(path: &Path) -> io::Result<bool> {
    match OpenOptions::new().read(true).write(true).open(path) {
        Ok(_) => Ok(true),
        Err(error) if is_read_only_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}
fn file_is_writable_with_control(
    path: &Path,
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<bool> {
    check_control(control)?;
    let writable = file_is_writable(path)?;
    check_control(control)?;
    Ok(writable)
}

fn is_read_only_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem
    )
}

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
        Err(error) if is_read_only_error(&error) => Ok(false),
        Err(error) => Err(error),
    }
}
fn probe_directory_write_with_control(
    directory: &Path,
    control: Option<&DiscoveryControl<'_>>,
) -> io::Result<bool> {
    check_control(control)?;
    let writable = probe_directory_write(directory)?;
    check_control(control)?;
    Ok(writable)
}

struct ExistingAncestorWalk {
    existing_path: PathBuf,
    resolved_existing_path: PathBuf,
    existing_identity: String,
    expected_target: PathBuf,
    missing_directories: Vec<PathBuf>,
}

impl ExistingAncestorWalk {
    fn new(target: &Path) -> io::Result<Self> {
        Self::new_with_control(target, None)
    }

    fn new_with_control(target: &Path, control: Option<&DiscoveryControl<'_>>) -> io::Result<Self> {
        let mut missing_directories = Vec::new();
        let mut candidate = target;
        loop {
            check_control(control)?;
            match fs::symlink_metadata(candidate) {
                Ok(metadata) => {
                    check_control(control)?;
                    let resolved_existing_path = candidate.canonicalize()?;
                    check_control(control)?;
                    if !fs::metadata(&resolved_existing_path)?.is_dir() {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "Target configuration 的现有父路径不是目录",
                        ));
                    }
                    let expected_target = if candidate == target && metadata.is_dir() {
                        resolved_existing_path.clone()
                    } else {
                        let suffix = target.strip_prefix(candidate).map_err(io::Error::other)?;
                        resolved_existing_path.join(suffix)
                    };
                    missing_directories.reverse();
                    return Ok(Self {
                        existing_path: candidate.to_path_buf(),
                        existing_identity: filesystem_identity(&resolved_existing_path)?,
                        resolved_existing_path,
                        expected_target,
                        missing_directories,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    missing_directories.push(candidate.to_path_buf());
                    candidate = candidate.parent().ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::NotFound,
                            "Target configuration 没有现有父目录",
                        )
                    })?;
                }
                Err(error) => return Err(error),
            }
        }
    }
}
#[cfg(unix)]
fn filesystem_identity(path: &Path) -> io::Result<String> {
    let metadata = fs::metadata(path)?;
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}
#[cfg(unix)]
fn filesystem_identity_from_file(file: &fs::File) -> io::Result<String> {
    let metadata = file.metadata()?;
    Ok(format!("unix:{}:{}", metadata.dev(), metadata.ino()))
}

#[cfg(windows)]
fn filesystem_identity(path: &Path) -> io::Result<String> {
    use windows_sys::Win32::{
        Foundation::{HANDLE, INVALID_HANDLE_VALUE},
        Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_SHARE_DELETE,
            FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle, OPEN_EXISTING,
        },
    };

    let wide_path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let raw_handle = unsafe {
        CreateFileW(
            wide_path.as_ptr(),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if raw_handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    let handle = unsafe { OwnedHandle::from_raw_handle(raw_handle) };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let success =
        unsafe { GetFileInformationByHandle(handle.as_raw_handle() as HANDLE, &mut information) };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(format!(
        "windows:{}:{file_index}",
        information.dwVolumeSerialNumber
    ))
}
#[cfg(windows)]
fn filesystem_identity_from_file(file: &fs::File) -> io::Result<String> {
    use windows_sys::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle},
    };

    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let success =
        unsafe { GetFileInformationByHandle(file.as_raw_handle() as HANDLE, &mut information) };
    if success == 0 {
        return Err(io::Error::last_os_error());
    }
    let file_index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Ok(format!(
        "windows:{}:{file_index}",
        information.dwVolumeSerialNumber
    ))
}
fn ensure_filesystem_identity(
    path: &Path,
    expected_resolved_path: &Path,
    expected_identity: &str,
) -> io::Result<()> {
    let resolved = path.canonicalize()?;
    if resolved == expected_resolved_path && filesystem_identity(&resolved)? == expected_identity {
        Ok(())
    } else {
        Err(io::Error::other(
            "Target configuration 文件系统身份在操作期间发生变化",
        ))
    }
}

fn creation_paths(
    ancestor: &ExistingAncestorWalk,
    models: &ConfigurationFileStatus,
    config: &ConfigurationFileStatus,
) -> Vec<String> {
    let mut paths: Vec<String> = ancestor
        .missing_directories
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    let target = ancestor
        .missing_directories
        .last()
        .map(PathBuf::as_path)
        .unwrap_or(&ancestor.existing_path);
    if matches!(models, ConfigurationFileStatus::Missing) {
        paths.push(target.join("models.yml").to_string_lossy().into_owned());
    }
    if matches!(config, ConfigurationFileStatus::Missing) {
        paths.push(target.join("config.yml").to_string_lossy().into_owned());
    }
    paths
}

fn discovery_token(
    target: &Path,
    resolved_path: Option<&str>,
    ancestor: &ExistingAncestorWalk,
    status: &TargetConfigurationStatus,
    models: &ConfigurationFileDiscovery,
    config: &ConfigurationFileDiscovery,
    create_paths: &[String],
) -> io::Result<String> {
    serde_json::to_string(&(
        target.to_string_lossy(),
        resolved_path,
        ancestor.expected_target.to_string_lossy(),
        &ancestor.existing_identity,
        status,
        models,
        config,
        create_paths,
    ))
    .map_err(io::Error::other)
}

fn unsafe_discovery(target: &Path, message: String) -> TargetConfigurationDiscovery {
    let file = |name: &str| ConfigurationFileDiscovery {
        canonical_path: target.join(name).to_string_lossy().into_owned(),
        resolved_path: None,
        status: ConfigurationFileStatus::Unsafe,
    };
    TargetConfigurationDiscovery {
        path: target.to_string_lossy().into_owned(),
        resolved_path: None,
        status: TargetConfigurationStatus::Unsafe,
        writable: false,
        models: file("models.yml"),
        config: file("config.yml"),
        recovery_notice: None,
        create_paths: Vec::new(),
        discovery_token: String::new(),
        warnings: Vec::new(),
        issue: Some(ConfigurationIssue {
            file_path: target.to_string_lossy().into_owned(),
            line: None,
            column: None,
            message,
        }),
    }
}

fn is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    if metadata.file_type().is_symlink() {
        return true;
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        return metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0;
    }
    #[cfg(not(windows))]
    false
}

fn combined_status(
    models: &ConfigurationFileStatus,
    config: &ConfigurationFileStatus,
    directory_writable: bool,
) -> TargetConfigurationStatus {
    let statuses = [models, config];
    if statuses
        .iter()
        .any(|status| matches!(status, ConfigurationFileStatus::Unsafe))
    {
        TargetConfigurationStatus::Unsafe
    } else if statuses
        .iter()
        .any(|status| matches!(status, ConfigurationFileStatus::ParseError))
    {
        TargetConfigurationStatus::ParseError
    } else if statuses
        .iter()
        .any(|status| matches!(status, ConfigurationFileStatus::LegacyJson))
    {
        TargetConfigurationStatus::MigrationRequired
    } else if statuses
        .iter()
        .any(|status| matches!(status, ConfigurationFileStatus::Missing))
    {
        TargetConfigurationStatus::CreationRequired
    } else if statuses.iter().any(|status| {
        matches!(
            status,
            ConfigurationFileStatus::AlternateOnly | ConfigurationFileStatus::ReadOnly
        )
    }) || !directory_writable
    {
        TargetConfigurationStatus::ReadOnly
    } else {
        TargetConfigurationStatus::Writable
    }
}

const MINIMAL_MODELS_YAML: &str = "providers: {}\n";
const MINIMAL_CONFIG_YAML: &str = "modelRoles: {}\n";

#[cfg(test)]
pub fn initialize_target_configuration(
    target: &Path,
    expectation: &TargetInitializationExpectation,
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_with_store(target, &transaction_root, expectation)
}

pub fn initialize_target_configuration_with_store(
    target: &Path,
    transaction_root: &Path,
    expectation: &TargetInitializationExpectation,
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    initialize_target_configuration_internal(
        target,
        transaction_root,
        expectation,
        None,
        (|| {}, || {}, || {}, || {}),
    )
}

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum InitializationFailurePoint {
    CorruptConfigBeforeValidation,
    AfterFirstCommit,
    CrashAfterDirectoryCreation,
    CrashBeforeDirectoryRename,
    CrashAfterDirectoryMarker,
    CrashAfterFirstCommit,
    CrashAfterCommitMarker,
}

#[cfg(test)]
pub(crate) fn initialize_target_configuration_with_failure(
    target: &Path,
    expectation: &TargetInitializationExpectation,
    failure: InitializationFailurePoint,
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_internal(
        target,
        &transaction_root,
        expectation,
        Some(failure),
        (|| {}, || {}, || {}, || {}),
    )
}

#[cfg(test)]
fn initialize_target_configuration_with_hook(
    target: &Path,
    expectation: &TargetInitializationExpectation,
    before_prepare: impl FnOnce(),
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_internal(
        target,
        &transaction_root,
        expectation,
        None,
        (before_prepare, || {}, || {}, || {}),
    )
}
#[cfg(test)]
fn initialize_target_configuration_with_commit_hook(
    target: &Path,
    expectation: &TargetInitializationExpectation,
    after_first_commit: impl FnOnce(),
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_internal(
        target,
        &transaction_root,
        expectation,
        None,
        (|| {}, || {}, after_first_commit, || {}),
    )
}
#[cfg(test)]
fn initialize_target_configuration_with_committed_hook(
    target: &Path,
    expectation: &TargetInitializationExpectation,
    after_committed_marker: impl FnOnce(),
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_internal(
        target,
        &transaction_root,
        expectation,
        None,
        (|| {}, || {}, || {}, after_committed_marker),
    )
}
#[cfg(test)]
fn initialize_target_configuration_with_directory_publish_hook(
    target: &Path,
    expectation: &TargetInitializationExpectation,
    after_directory_publish: impl FnMut(),
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let transaction_root = test_transaction_root(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    initialize_target_configuration_internal(
        target,
        &transaction_root,
        expectation,
        None,
        (|| {}, after_directory_publish, || {}, || {}),
    )
}

fn initialize_target_configuration_internal(
    target: &Path,
    transaction_root: &Path,
    expectation: &TargetInitializationExpectation,
    failure: Option<InitializationFailurePoint>,
    hooks: (impl FnOnce(), impl FnMut(), impl FnOnce(), impl FnOnce()),
) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
    let (before_prepare, mut after_directory_publish, after_first_commit, after_committed_marker) =
        hooks;
    let _ = recover_interrupted_initialization(target, transaction_root)
        .map_err(|error| TargetInitializationError::new(error, true))?;
    let initial = discover_target_configuration_internal(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    if initial.status == TargetConfigurationStatus::Writable
        && expectation.create_paths.is_empty()
        && initial.discovery_token == expectation.discovery_token
    {
        return Ok(initial);
    }
    ensure_creation_plan_unchanged(&initial, expectation)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    let ancestor = ExistingAncestorWalk::new(target)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    let transaction_id = INITIALIZATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    fs::create_dir_all(transaction_root)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    let trusted_transaction_root = transaction_root
        .canonicalize()
        .map_err(|error| TargetInitializationError::new(error, false))?;
    let manifest_path = trusted_transaction_root.join(initialization_manifest_name(target));
    let entries = [
        (
            "models.yml",
            initial.models.status == ConfigurationFileStatus::Missing,
        ),
        (
            "config.yml",
            initial.config.status == ConfigurationFileStatus::Missing,
        ),
    ]
    .into_iter()
    .filter(|(_, missing)| *missing)
    .map(|(name, _)| {
        let expected_contents = match name {
            "models.yml" => MINIMAL_MODELS_YAML.as_bytes(),
            "config.yml" => MINIMAL_CONFIG_YAML.as_bytes(),
            _ => unreachable!("fixed initialization entry"),
        };
        InitializationEntry {
            staging: ancestor.expected_target.join(format!(
                ".omp-switch-init-{}-{transaction_id}-{name}.tmp",
                std::process::id()
            )),
            destination: ancestor.expected_target.join(name),
            expected_final_hash: content_hash(expected_contents),
        }
    })
    .collect();
    let mut manifest = InitializationManifest {
        logical_target: target.to_path_buf(),
        phase: InitializationPhase::Prepared,
        transaction_id,
        process_id: std::process::id(),
        resolved_target: ancestor.expected_target.clone(),
        existing_ancestor: ancestor.resolved_existing_path.clone(),
        existing_ancestor_identity: ancestor.existing_identity.clone(),
        resolved_target_identity: ancestor
            .missing_directories
            .is_empty()
            .then(|| ancestor.existing_identity.clone()),
        target_existed: ancestor.missing_directories.is_empty(),
        created_directories: resolved_missing_directories(&ancestor)
            .map_err(|error| TargetInitializationError::new(error, false))?,
        created_directory_identities: Vec::new(),
        directory_creation_in_progress: None,
        entries,
    };
    persist_initialization_manifest(&manifest_path, &manifest)
        .map_err(|error| TargetInitializationError::new(error, false))?;
    before_prepare();
    let mut after_first_commit = Some(after_first_commit);

    let operation = (|| -> io::Result<TargetConfigurationDiscovery> {
        ensure_filesystem_identity(
            &ancestor.existing_path,
            &ancestor.resolved_existing_path,
            &ancestor.existing_identity,
        )?;
        let opened_target = open_or_create_target_directory(
            &ancestor,
            &manifest_path,
            &mut manifest,
            failure,
            &mut after_directory_publish,
        )?;
        let target_directory = &opened_target.target;
        if failure == Some(InitializationFailurePoint::CrashAfterDirectoryCreation) {
            return Err(io::Error::other("injected crash after directory creation"));
        }
        let target_identity =
            filesystem_identity_from_file(&target_directory.try_clone()?.into_std_file())?;
        manifest.resolved_target_identity = Some(target_identity.clone());
        persist_initialization_manifest(&manifest_path, &manifest)?;
        ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)?;
        let expected_after_directories: Vec<String> = expectation
            .create_paths
            .iter()
            .filter(|path| {
                !ancestor
                    .missing_directories
                    .iter()
                    .any(|directory| Path::new(path.as_str()) == directory)
            })
            .cloned()
            .collect();
        let after_directory_creation = discover_target_configuration_internal(target)?;
        ensure_create_paths_unchanged(&after_directory_creation, &expected_after_directories)?;
        ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)?;

        for entry in &manifest.entries {
            let name = entry
                .destination
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "配置文件名无效"))?;
            let contents = match name {
                "models.yml" => MINIMAL_MODELS_YAML.as_bytes(),
                "config.yml"
                    if failure
                        == Some(InitializationFailurePoint::CorruptConfigBeforeValidation) =>
                {
                    b"modelRoles: [\n".as_slice()
                }
                "config.yml" => MINIMAL_CONFIG_YAML.as_bytes(),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "初始化事务包含未知目标",
                    ));
                }
            };
            let staging_name = entry
                .staging
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "配置暂存文件名无效"))?;
            let mut options = CapOpenOptions::new();
            options.write(true).create_new(true);
            let mut file = target_directory.open_with(staging_name, &options)?;
            file.write_all(contents)?;
            file.sync_all()?;
            let serialized = target_directory.read(staging_name)?;
            serde_yaml::from_slice::<serde_yaml::Value>(&serialized).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("最小配置验证失败：{error}"),
                )
            })?;
        }

        let before_commit = discover_target_configuration_internal(target)?;
        ensure_create_paths_unchanged(&before_commit, &expected_after_directories)?;
        ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)?;
        for (index, entry) in manifest.entries.iter().enumerate() {
            let staging_name = entry
                .staging
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "配置暂存文件名无效"))?;
            let destination_name = entry
                .destination
                .file_name()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "配置目标文件名无效"))?;
            target_directory.hard_link(staging_name, target_directory, destination_name)?;
            if index == 0
                && let Some(after_first_commit) = after_first_commit.take()
            {
                after_first_commit();
            }
            if index == 0
                && matches!(
                    failure,
                    Some(
                        InitializationFailurePoint::AfterFirstCommit
                            | InitializationFailurePoint::CrashAfterFirstCommit
                    )
                )
            {
                return Err(io::Error::other("injected initialization commit failure"));
            }
        }
        if let Err(error) =
            ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)
        {
            return Err(post_publication_identity_error(error));
        }

        let final_discovery = match discover_target_configuration_internal(target) {
            Ok(discovery) => discovery,
            Err(error) => {
                if let Err(identity_error) =
                    ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)
                {
                    return Err(post_publication_identity_error(identity_error));
                }
                return Err(error);
            }
        };
        if !matches!(
            final_discovery.status,
            TargetConfigurationStatus::Writable | TargetConfigurationStatus::ReadOnly
        ) || !final_discovery.create_paths.is_empty()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "创建后的 Target configuration 未通过重新发现和解析",
            ));
        }
        if let Err(error) =
            ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)
        {
            return Err(post_publication_identity_error(error));
        }
        manifest.phase = InitializationPhase::Committed;
        persist_initialization_manifest(&manifest_path, &manifest)?;
        after_committed_marker();
        if failure == Some(InitializationFailurePoint::CrashAfterCommitMarker) {
            return Err(io::Error::other("injected crash after committed manifest"));
        }
        cleanup_committed_initialization_with_handles(
            &opened_target.created_directories,
            target_directory,
            &manifest,
        )?;
        if let Err(error) =
            ensure_filesystem_identity(target, &ancestor.expected_target, &target_identity)
        {
            return Err(post_publication_identity_error(error));
        }
        fs::remove_file(&manifest_path)?;
        Ok(final_discovery)
    })();

    match operation {
        Ok(discovery) => Ok(discovery),
        Err(error) if manifest_filesystem_identity_is_unstable(&manifest) => {
            Err(TargetInitializationError::new(error, true))
        }
        Err(error) if is_post_publication_identity_error(&error) => {
            Err(TargetInitializationError::new(error, true))
        }
        Err(error) if manifest.phase == InitializationPhase::Committed => {
            Err(TargetInitializationError::new(error, true))
        }
        Err(error)
            if matches!(
                failure,
                Some(
                    InitializationFailurePoint::CrashAfterDirectoryCreation
                        | InitializationFailurePoint::CrashBeforeDirectoryRename
                        | InitializationFailurePoint::CrashAfterDirectoryMarker
                        | InitializationFailurePoint::CrashAfterFirstCommit
                        | InitializationFailurePoint::CrashAfterCommitMarker
                )
            ) =>
        {
            Err(TargetInitializationError::new(error, true))
        }
        Err(error) => match recover_manifest(&manifest_path, &manifest) {
            Ok(_) => Err(TargetInitializationError::new(error, false)),
            Err(recovery_error) => Err(TargetInitializationError::new(
                io::Error::new(error.kind(), format!("{error}；{recovery_error}")),
                true,
            )),
        },
    }
}

fn manifest_filesystem_identity_is_unstable(manifest: &InitializationManifest) -> bool {
    let target_is_unstable = manifest
        .resolved_target_identity
        .as_deref()
        .is_some_and(|identity| {
            ensure_filesystem_identity(
                &manifest.logical_target,
                &manifest.resolved_target,
                identity,
            )
            .is_err()
        });
    target_is_unstable
        || manifest.directory_creation_in_progress.is_some()
        || manifest.created_directory_identities.len() > manifest.created_directories.len()
        || manifest
            .created_directories
            .iter()
            .zip(&manifest.created_directory_identities)
            .enumerate()
            .any(|(index, (directory, identity))| {
                !matches!(
                    recorded_directory_location(manifest, index, directory, identity),
                    Ok(Some(_))
                )
            })
}

fn ensure_creation_plan_unchanged(
    discovery: &TargetConfigurationDiscovery,
    expectation: &TargetInitializationExpectation,
) -> io::Result<()> {
    if discovery.status == TargetConfigurationStatus::CreationRequired
        && discovery.create_paths == expectation.create_paths
        && discovery.discovery_token == expectation.discovery_token
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Target configuration 在确认后发生变化",
        ))
    }
}

fn ensure_create_paths_unchanged(
    discovery: &TargetConfigurationDiscovery,
    expected_create_paths: &[String],
) -> io::Result<()> {
    if discovery.status == TargetConfigurationStatus::CreationRequired
        && discovery.create_paths == expected_create_paths
    {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "Target configuration 在初始化期间发生变化",
        ))
    }
}

fn directory_marker_name(manifest: &InitializationManifest) -> String {
    format!(
        ".omp-switch-init-{}-{}.marker",
        manifest.process_id, manifest.transaction_id
    )
}

fn directory_staging_name(manifest: &InitializationManifest, index: usize) -> String {
    format!(
        ".omp-switch-init-{}-{}-dir-{index}.tmp",
        manifest.process_id, manifest.transaction_id
    )
}

fn directory_marker_contents(manifest: &InitializationManifest) -> String {
    format!(
        "{}\n{}\n",
        manifest.transaction_id,
        manifest.logical_target.display()
    )
}

struct OpenedTargetDirectory {
    created_directories: Vec<Dir>,
    target: Dir,
}

fn open_or_create_target_directory(
    ancestor: &ExistingAncestorWalk,
    manifest_path: &Path,
    manifest: &mut InitializationManifest,
    failure: Option<InitializationFailurePoint>,
    after_directory_publish: &mut impl FnMut(),
) -> io::Result<OpenedTargetDirectory> {
    let ancestor_directory =
        Dir::open_ambient_dir(&ancestor.resolved_existing_path, ambient_authority())?;
    let mut current = ancestor_directory.try_clone()?;
    if filesystem_identity_from_file(&current.try_clone()?.into_std_file())?
        != ancestor.existing_identity
    {
        return Err(io::Error::other(
            "Target configuration 现有父目录身份在打开句柄前发生变化",
        ));
    }
    let marker_name = directory_marker_name(manifest);
    let marker_contents = directory_marker_contents(manifest);
    let mut created_directories = Vec::with_capacity(manifest.created_directories.len());
    for index in 0..manifest.created_directories.len() {
        let destination = manifest.created_directories[index].clone();
        let destination_name = destination
            .file_name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "待创建目录没有文件名"))?;
        let staging_name = directory_staging_name(manifest, index);
        manifest.directory_creation_in_progress = Some(index);
        persist_initialization_manifest(manifest_path, manifest)?;
        current.create_dir(&staging_name)?;
        let staged = current.open_dir(&staging_name)?;
        let mut marker_options = CapOpenOptions::new();
        marker_options.write(true).create_new(true);
        let mut marker = staged.open_with(&marker_name, &marker_options)?;
        marker.write_all(marker_contents.as_bytes())?;
        marker.sync_all()?;
        drop(marker);
        if failure == Some(InitializationFailurePoint::CrashAfterDirectoryMarker) {
            return Err(io::Error::other(
                "injected crash after directory marker persistence",
            ));
        }
        let staged_identity = filesystem_identity_from_file(&staged.try_clone()?.into_std_file())?;
        manifest
            .created_directory_identities
            .push(staged_identity.clone());
        manifest.directory_creation_in_progress = None;
        persist_initialization_manifest(manifest_path, manifest)?;
        if failure == Some(InitializationFailurePoint::CrashBeforeDirectoryRename) {
            return Err(io::Error::other(
                "injected crash before directory capability rename",
            ));
        }
        drop(staged);
        current.rename(&staging_name, &current, destination_name)?;
        let published = current.open_dir(Path::new(destination_name))?;
        if filesystem_identity_from_file(&published.try_clone()?.into_std_file())?
            != staged_identity
        {
            return Err(io::Error::other("初始化目录发布后的文件系统身份发生变化"));
        }
        after_directory_publish();
        current = published;
        created_directories.push(current.try_clone()?);
    }
    Ok(OpenedTargetDirectory {
        created_directories,
        target: current,
    })
}

fn resolved_missing_directories(ancestor: &ExistingAncestorWalk) -> io::Result<Vec<PathBuf>> {
    ancestor
        .missing_directories
        .iter()
        .map(|directory| {
            let suffix = directory
                .strip_prefix(&ancestor.existing_path)
                .map_err(io::Error::other)?;
            Ok(ancestor.resolved_existing_path.join(suffix))
        })
        .collect()
}

fn initialization_manifest_name(target: &Path) -> String {
    format!(
        ".omp-switch-init-transaction-{:016x}.json",
        stable_path_hash(target)
    )
}

#[cfg(test)]
fn test_transaction_root(target: &Path) -> io::Result<PathBuf> {
    Ok(std::env::temp_dir()
        .canonicalize()?
        .join("omp-switch-test-transactions")
        .join(format!("{:016x}", stable_path_hash(target))))
}

fn find_initialization_manifest(
    target: &Path,
    transaction_root: &Path,
) -> io::Result<Option<PathBuf>> {
    if !path_entry_exists(transaction_root)? {
        return Ok(None);
    }
    let trusted_root = transaction_root.canonicalize()?;
    let manifest = trusted_root.join(initialization_manifest_name(target));
    Ok(path_entry_exists(&manifest)?.then_some(manifest))
}

fn stable_path_hash(path: &Path) -> u64 {
    path.to_string_lossy()
        .as_bytes()
        .iter()
        .fold(0xcbf29ce484222325_u64, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(0x100000001b3)
        })
}

fn content_hash(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[usize::from(byte >> 4)] as char);
        encoded.push(HEX[usize::from(byte & 0x0f)] as char);
    }
    encoded
}

fn file_hash(path: &Path) -> io::Result<String> {
    Ok(content_hash(&fs::read(path)?))
}

fn persist_initialization_manifest(
    path: &Path,
    manifest: &InitializationManifest,
) -> io::Result<()> {
    let bytes = serde_json::to_vec(manifest).map_err(io::Error::other)?;
    let mut file = atomic_write_file::AtomicWriteFile::options().open(path)?;
    file.write_all(&bytes)?;
    file.commit()
}
fn recover_interrupted_initialization(
    target: &Path,
    transaction_root: &Path,
) -> io::Result<Option<String>> {
    let Some(manifest_path) = find_initialization_manifest(target, transaction_root)? else {
        return Ok(None);
    };
    let manifest: InitializationManifest = match serde_json::from_slice(&fs::read(&manifest_path)?)
    {
        Ok(manifest) => manifest,
        Err(error) => {
            quarantine_invalid_manifest(&manifest_path)?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("初始化事务清单无法解析，已隔离：{error}"),
            ));
        }
    };
    if let Err(error) = validate_initialization_manifest(target, &manifest) {
        quarantine_invalid_manifest(&manifest_path)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("初始化事务清单验证失败，已隔离：{error}"),
        ));
    }
    if let Err(error) = ensure_filesystem_identity(
        &manifest.existing_ancestor,
        &manifest.existing_ancestor,
        &manifest.existing_ancestor_identity,
    ) {
        quarantine_invalid_manifest(&manifest_path)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("事务现有父目录身份已变化，清单已隔离：{error}"),
        ));
    }
    if let Some(index) = manifest.directory_creation_in_progress {
        quarantine_invalid_manifest(&manifest_path)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("事务第 {index} 个目录的创建身份尚未确认，清单已隔离"),
        ));
    }
    for (index, (directory, identity)) in manifest
        .created_directories
        .iter()
        .zip(&manifest.created_directory_identities)
        .enumerate()
    {
        if recorded_directory_location(&manifest, index, directory, identity)?.is_none() {
            quarantine_invalid_manifest(&manifest_path)?;
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "事务创建目录身份已变化，清单已隔离",
            ));
        }
    }
    if let Some(identity) = manifest.resolved_target_identity.as_deref()
        && let Err(error) = ensure_filesystem_identity(
            &manifest.resolved_target,
            &manifest.resolved_target,
            identity,
        )
    {
        quarantine_invalid_manifest(&manifest_path)?;
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("事务目标身份已变化，清单已隔离：{error}"),
        ));
    }
    recover_manifest(&manifest_path, &manifest).map(Some)
}

fn validate_initialization_manifest(
    target: &Path,
    manifest: &InitializationManifest,
) -> io::Result<()> {
    if manifest.logical_target != target {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "逻辑 Target configuration 不匹配",
        ));
    }
    if !is_normalized_absolute(&manifest.resolved_target) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "事务真实目标不是规范绝对路径",
        ));
    }
    if !is_normalized_absolute(&manifest.existing_ancestor)
        || !manifest
            .resolved_target
            .starts_with(&manifest.existing_ancestor)
        || manifest.existing_ancestor_identity.is_empty()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "事务现有父目录身份无效",
        ));
    }
    if manifest.target_existed {
        if !manifest.created_directories.is_empty()
            || !manifest.created_directory_identities.is_empty()
            || manifest.directory_creation_in_progress.is_some()
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "已存在目标不应包含待恢复目录",
            ));
        }
    } else {
        if let Some(index) = manifest.directory_creation_in_progress
            && (index >= manifest.created_directories.len()
                || manifest.created_directory_identities.len() != index)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "事务进行中的创建目录索引无效",
            ));
        }
        if manifest.created_directory_identities.len() > manifest.created_directories.len()
            || manifest
                .created_directory_identities
                .iter()
                .any(String::is_empty)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "事务创建目录身份无效",
            ));
        }
        if manifest.created_directories.last() != Some(&manifest.resolved_target) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "待恢复目录链没有终止于真实目标",
            ));
        }
        for (index, directory) in manifest.created_directories.iter().enumerate() {
            if !is_normalized_absolute(directory) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "待恢复目录不是规范绝对路径",
                ));
            }
            if index > 0
                && directory.parent()
                    != manifest
                        .created_directories
                        .get(index - 1)
                        .map(PathBuf::as_path)
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "待恢复目录链不连续",
                ));
            }
        }
    }
    if manifest.entries.is_empty() || manifest.entries.len() > 2 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "事务目标数量无效",
        ));
    }
    let mut seen_models = false;
    let mut seen_config = false;
    for entry in &manifest.entries {
        let name = entry
            .destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "事务目标文件名无效"))?;
        let (seen, expected_contents) = match name {
            "models.yml" => (&mut seen_models, MINIMAL_MODELS_YAML.as_bytes()),
            "config.yml" => (&mut seen_config, MINIMAL_CONFIG_YAML.as_bytes()),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "事务包含范围外目标",
                ));
            }
        };
        if *seen {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "事务包含重复目标",
            ));
        }
        *seen = true;
        let expected_destination = manifest.resolved_target.join(name);
        let expected_staging = manifest.resolved_target.join(format!(
            ".omp-switch-init-{}-{}-{name}.tmp",
            manifest.process_id, manifest.transaction_id
        ));
        if entry.destination != expected_destination
            || entry.staging != expected_staging
            || entry.expected_final_hash != content_hash(expected_contents)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "事务目标路径、暂存路径或最终 Hash 不匹配",
            ));
        }
    }
    if !manifest.target_existed && !(seen_models && seen_config) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "新建 Target configuration 的事务目标不完整",
        ));
    }
    Ok(())
}

fn is_normalized_absolute(path: &Path) -> bool {
    path.is_absolute()
        && !path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
}

fn quarantine_invalid_manifest(manifest_path: &Path) -> io::Result<()> {
    let sequence = INITIALIZATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let file_name = manifest_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(".omp-switch-init-transaction.json");
    let quarantine = manifest_path.with_file_name(format!("{file_name}.invalid-{sequence}"));
    fs::rename(manifest_path, quarantine)
}

fn recover_manifest(path: &Path, manifest: &InitializationManifest) -> io::Result<String> {
    match manifest.phase {
        InitializationPhase::Prepared => {
            let preserved_external = rollback_prepared_initialization(path, manifest)?;
            Ok(if preserved_external {
                "已清理上次中断事务的应用暂存内容；外部修改或替换的文件已原位保留。".to_owned()
            } else {
                "已回滚上次中断的 Target configuration 初始化；未保留部分创建结果。".to_owned()
            })
        }
        InitializationPhase::Committed => {
            cleanup_committed_initialization(path, manifest)?;
            Ok(
                "上次 Target configuration 初始化已提交；已清理事务暂存文件并保留当前配置。"
                    .to_owned(),
            )
        }
    }
}

fn rollback_prepared_initialization(
    manifest_path: &Path,
    manifest: &InitializationManifest,
) -> io::Result<bool> {
    let mut failures = Vec::new();
    let mut preserved_external = false;
    for entry in manifest.entries.iter().rev() {
        let staging_exists = path_entry_exists(&entry.staging)?;
        if path_entry_exists(&entry.destination)? {
            if !staging_exists {
                preserved_external = true;
            } else if same_file::is_same_file(&entry.staging, &entry.destination)? {
                if file_hash(&entry.destination)? == entry.expected_final_hash {
                    if let Err(error) = fs::remove_file(&entry.destination) {
                        failures.push(format!("{}: {error}", entry.destination.display()));
                    }
                } else {
                    preserved_external = true;
                }
            } else {
                preserved_external = true;
            }
        }
        if staging_exists
            && let Err(error) = fs::remove_file(&entry.staging)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", entry.staging.display()));
        }
    }
    let marker_name = directory_marker_name(manifest);
    let marker_contents = directory_marker_contents(manifest);
    for (index, directory) in manifest.created_directories.iter().enumerate().rev() {
        let Some(identity) = manifest.created_directory_identities.get(index) else {
            continue;
        };
        match recorded_directory_location(manifest, index, directory, identity)? {
            Some(location) => {
                preserved_external |= cleanup_transaction_directory(
                    &location,
                    &marker_name,
                    &marker_contents,
                    true,
                    &mut failures,
                )?;
            }
            None => preserved_external = true,
        }
    }
    finish_manifest_cleanup(manifest_path, failures)?;
    Ok(preserved_external)
}
fn directory_staging_path(
    manifest: &InitializationManifest,
    index: usize,
    destination: &Path,
) -> io::Result<PathBuf> {
    let parent = destination
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "待恢复目录没有父目录"))?;
    Ok(parent.join(directory_staging_name(manifest, index)))
}
fn recorded_directory_location(
    manifest: &InitializationManifest,
    index: usize,
    destination: &Path,
    identity: &str,
) -> io::Result<Option<PathBuf>> {
    if ensure_filesystem_identity(destination, destination, identity).is_ok() {
        return Ok(Some(destination.to_path_buf()));
    }
    let staging = directory_staging_path(manifest, index, destination)?;
    if ensure_filesystem_identity(&staging, &staging, identity).is_ok() {
        Ok(Some(staging))
    } else {
        Ok(None)
    }
}

fn cleanup_transaction_directory(
    directory: &Path,
    marker_name: &str,
    marker_contents: &str,
    require_marker: bool,
    failures: &mut Vec<String>,
) -> io::Result<bool> {
    if !path_entry_exists(directory)? {
        return Ok(false);
    }
    if !fs::metadata(directory)?.is_dir() {
        return Ok(true);
    }
    let marker = directory.join(marker_name);
    if path_entry_exists(&marker)? {
        if fs::read_to_string(&marker)? != marker_contents {
            return Ok(true);
        }
        if let Err(error) = fs::remove_file(&marker)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", marker.display()));
            return Ok(false);
        }
    } else if require_marker {
        return Ok(true);
    }
    match fs::remove_dir(directory) {
        Ok(()) => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(true),
        Err(error) => {
            failures.push(format!("{}: {error}", directory.display()));
            Ok(false)
        }
    }
}

fn cleanup_committed_initialization_with_handles(
    created_directory_handles: &[Dir],
    target_directory: &Dir,
    manifest: &InitializationManifest,
) -> io::Result<()> {
    let mut failures = Vec::new();
    for entry in &manifest.entries {
        let Some(staging_name) = entry.staging.file_name() else {
            failures.push(format!("配置暂存文件名无效：{}", entry.staging.display()));
            continue;
        };
        if let Err(error) = target_directory.remove_file(staging_name)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", entry.staging.display()));
        }
    }

    let marker_name = directory_marker_name(manifest);
    let marker_contents = directory_marker_contents(manifest);
    if created_directory_handles.len() != manifest.created_directories.len() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "初始化事务创建目录句柄数量不匹配",
        ));
    }
    for (directory, opened) in manifest
        .created_directories
        .iter()
        .zip(created_directory_handles)
        .rev()
    {
        match opened.read_to_string(&marker_name) {
            Ok(contents) if contents == marker_contents => {
                if let Err(error) = opened.remove_file(&marker_name)
                    && error.kind() != io::ErrorKind::NotFound
                {
                    failures.push(format!(
                        "{}: {error}",
                        directory.join(&marker_name).display()
                    ));
                }
            }
            Ok(_) => failures.push(format!(
                "{}: 初始化目录标记内容已变化",
                directory.join(&marker_name).display()
            )),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => failures.push(format!(
                "{}: {error}",
                directory.join(&marker_name).display()
            )),
        }
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "初始化事务提交清理未完成：{}",
            failures.join("；")
        )))
    }
}

fn cleanup_committed_initialization(
    manifest_path: &Path,
    manifest: &InitializationManifest,
) -> io::Result<()> {
    let mut failures = Vec::new();
    for entry in &manifest.entries {
        if let Err(error) = fs::remove_file(&entry.staging)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", entry.staging.display()));
        }
    }
    let marker_name = directory_marker_name(manifest);
    let marker_contents = directory_marker_contents(manifest);
    for (index, directory) in manifest.created_directories.iter().enumerate().rev() {
        let marker = directory.join(&marker_name);
        if path_entry_exists(&marker)?
            && fs::read_to_string(&marker)? == marker_contents
            && let Err(error) = fs::remove_file(&marker)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", marker.display()));
        }
        let staging = directory_staging_path(manifest, index, directory)?;
        let _ = cleanup_transaction_directory(
            &staging,
            &marker_name,
            &marker_contents,
            false,
            &mut failures,
        )?;
    }
    finish_manifest_cleanup(manifest_path, failures)
}

fn finish_manifest_cleanup(manifest_path: &Path, failures: Vec<String>) -> io::Result<()> {
    if failures.is_empty() {
        fs::remove_file(manifest_path)
    } else {
        Err(io::Error::other(format!(
            "初始化事务恢复未完成：{}",
            failures.join("；")
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, io, path::Path};

    use tempfile::tempdir;

    use super::{
        ConfigurationFileStatus, InitializationFailurePoint, InitializationManifest,
        InitializationPhase, TargetConfigurationStatus, TargetInitializationExpectation,
        discover_target_configuration, find_initialization_manifest,
        initialize_target_configuration, initialize_target_configuration_with_commit_hook,
        initialize_target_configuration_with_committed_hook,
        initialize_target_configuration_with_directory_publish_hook,
        initialize_target_configuration_with_failure, initialize_target_configuration_with_hook,
        test_transaction_root,
    };

    fn expectation(target: &Path) -> TargetInitializationExpectation {
        let discovery = discover_target_configuration(target).unwrap();
        TargetInitializationExpectation {
            create_paths: discovery.create_paths,
            discovery_token: discovery.discovery_token,
        }
    }

    #[test]
    fn discovers_canonical_yml_and_warns_about_untouched_yaml() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
        fs::write(target.join("models.yaml"), "providers: {}\n").unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            discovery.models.status,
            ConfigurationFileStatus::CanonicalWithAlternate
        );
        assert_eq!(discovery.config.status, ConfigurationFileStatus::Normal);
        assert!(
            discovery
                .warnings
                .iter()
                .any(|warning| warning.contains("models.yaml") && warning.contains("不会被修改"))
        );
    }

    #[test]
    fn yaml_only_configuration_is_read_only() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yaml"), "providers: {}\n").unwrap();
        fs::write(target.join("config.yaml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::ReadOnly);
        assert_eq!(
            discovery.models.status,
            ConfigurationFileStatus::AlternateOnly
        );
        assert_eq!(
            discovery.config.status,
            ConfigurationFileStatus::AlternateOnly
        );
        assert!(discovery.create_paths.is_empty());
    }

    #[test]
    fn alternate_yaml_with_a_missing_canonical_file_requires_creation() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yaml"), "providers: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(
            discovery.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert_eq!(
            discovery.models.status,
            ConfigurationFileStatus::AlternateOnly
        );
        assert_eq!(discovery.config.status, ConfigurationFileStatus::Missing);
        assert_eq!(
            discovery.create_paths,
            vec![target.join("config.yml").to_string_lossy().into_owned()]
        );
    }

    #[test]
    fn legacy_json_without_supported_yaml_requires_official_migration() {
        let root = tempdir().unwrap();

        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.json"), "{\"providers\":{}}\n").unwrap();
        fs::write(target.join("settings.json"), "{\"modelRoles\":{}}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(
            discovery.status,
            TargetConfigurationStatus::MigrationRequired
        );
        assert_eq!(discovery.models.status, ConfigurationFileStatus::LegacyJson);
        assert_eq!(discovery.config.status, ConfigurationFileStatus::LegacyJson);
        assert!(discovery.create_paths.is_empty());
    }
    #[test]
    fn abnormal_legacy_json_entry_is_reported_unsafe() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir_all(target.join("models.json")).unwrap();
        fs::write(target.join("settings.json"), "{\"modelRoles\":{}}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Unsafe);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::Unsafe);
        assert!(discovery.issue.unwrap().file_path.ends_with("models.json"));
    }

    #[test]
    fn creates_the_missing_canonical_file_then_returns_yaml_read_only_state() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yaml"), "providers: {}\n").unwrap();

        let discovery = initialize_target_configuration(&target, &expectation(&target)).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::ReadOnly);
        assert_eq!(
            fs::read(target.join("config.yml")).unwrap(),
            b"modelRoles: {}\n"
        );
        assert!(!target.join("models.yml").exists());
    }

    #[test]
    fn missing_directory_lists_every_canonical_path_for_confirmation() {
        let root = tempdir().unwrap();
        let missing_parent = root.path().join("missing-parent");
        let target = missing_parent.join("agent");

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(
            discovery.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert_eq!(
            discovery.create_paths,
            vec![
                missing_parent.to_string_lossy().into_owned(),
                target.to_string_lossy().into_owned(),
                target.join("models.yml").to_string_lossy().into_owned(),
                target.join("config.yml").to_string_lossy().into_owned(),
            ]
        );
    }
    #[cfg(unix)]
    #[test]
    fn missing_target_reports_real_path_below_symlink_ancestor() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_parent = root.path().join("real-parent");
        let linked_parent = root.path().join("linked-parent");
        fs::create_dir(&real_parent).unwrap();
        symlink(&real_parent, &linked_parent).unwrap();
        let target = linked_parent.join("agent");

        let discovery = discover_target_configuration(&target).unwrap();
        let expected = real_parent.canonicalize().unwrap().join("agent");

        assert_eq!(
            discovery.resolved_path.as_deref(),
            Some(expected.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn malformed_yaml_reports_file_line_and_column_without_becoming_writable() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), "providers:\n  broken: [\n").unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::ParseError);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::ParseError);
        let issue = discovery.issue.unwrap();
        assert!(issue.file_path.ends_with("models.yml"));
        assert!(issue.line.is_some());
        assert!(issue.column.is_some());
        assert!(!issue.message.is_empty());
    }
    #[test]
    fn invalid_utf8_yaml_is_reported_as_a_file_parse_error() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), [0xff, 0xfe, 0xfd]).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::ParseError);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::ParseError);
        assert!(discovery.issue.unwrap().file_path.ends_with("models.yml"));
    }

    #[test]
    fn configuration_directory_entry_is_reported_unsafe() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir_all(target.join("models.yml")).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Unsafe);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::Unsafe);
        assert!(discovery.issue.unwrap().message.contains("不是普通文件"));
    }

    #[cfg(unix)]
    #[test]
    fn configuration_socket_entry_is_reported_unsafe_without_blocking() {
        use std::os::unix::net::UnixListener;

        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        let _listener = UnixListener::bind(target.join("models.yml")).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Unsafe);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::Unsafe);
    }

    #[cfg(unix)]
    #[test]
    fn resolves_directory_and_file_symlinks_to_real_targets() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real = root.path().join("real-agent");
        let linked = root.path().join("linked-agent");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("real-models.yml"), "providers: {}\n").unwrap();
        symlink(real.join("real-models.yml"), real.join("models.yml")).unwrap();
        fs::write(real.join("config.yml"), "modelRoles: {}\n").unwrap();
        symlink(&real, &linked).unwrap();

        let discovery = discover_target_configuration(&linked).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            discovery.resolved_path.as_deref(),
            Some(real.canonicalize().unwrap().to_string_lossy().as_ref())
        );
        assert_eq!(
            discovery.models.resolved_path.as_deref(),
            Some(
                real.join("real-models.yml")
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .as_ref()
            )
        );
    }
    #[cfg(unix)]
    #[test]
    fn link_loop_is_reported_unsafe_instead_of_followed() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        symlink(&target, &target).unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Unsafe);
        assert!(!discovery.writable);
        assert!(discovery.issue.unwrap().message.contains("链接"));
    }

    #[cfg(unix)]
    #[test]
    fn configuration_file_link_loop_is_unsafe() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        symlink(target.join("models.yml"), target.join("models.yml")).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let discovery = discover_target_configuration(&target).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Unsafe);
        assert_eq!(discovery.models.status, ConfigurationFileStatus::Unsafe);
        assert!(discovery.issue.unwrap().message.contains("链接"));
    }

    #[cfg(windows)]
    #[test]
    fn resolves_directory_junction_reparse_point() {
        use std::process::Command;

        let root = tempdir().unwrap();
        let real = root.path().join("real-agent");
        let junction = root.path().join("junction-agent");
        fs::create_dir(&real).unwrap();
        fs::write(real.join("models.yml"), "providers: {}\n").unwrap();
        fs::write(real.join("config.yml"), "modelRoles: {}\n").unwrap();
        let junction_result = Command::new("cmd.exe")
            .args(["/C", "mklink", "/J"])
            .arg(&junction)
            .arg(&real)
            .output()
            .unwrap();
        assert!(
            junction_result.status.success(),
            "mklink failed: {}",
            String::from_utf8_lossy(&junction_result.stderr)
        );

        let discovery = discover_target_configuration(&junction).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            discovery.resolved_path.as_deref(),
            Some(real.canonicalize().unwrap().to_string_lossy().as_ref())
        );
    }

    #[test]
    fn atomically_creates_and_reparses_minimal_configuration() {
        let root = tempdir().unwrap();
        let target = root.path().join("nested").join("agent");

        let discovery = initialize_target_configuration(&target, &expectation(&target)).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            fs::read(target.join("models.yml")).unwrap(),
            b"providers: {}\n"
        );
        assert_eq!(
            fs::read(target.join("config.yml")).unwrap(),
            b"modelRoles: {}\n"
        );
        let mut entries: Vec<_> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        entries.sort();
        assert_eq!(entries, ["config.yml", "models.yml"]);
    }

    #[test]
    fn validation_failure_leaves_no_partial_directory_or_files() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");

        let result = initialize_target_configuration_with_failure(
            &target,
            &expectation(&target),
            InitializationFailurePoint::CorruptConfigBeforeValidation,
        );

        assert!(result.is_err());
        assert!(!target.exists());
    }

    #[test]
    fn commit_failure_rolls_back_every_created_file() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();

        let result = initialize_target_configuration_with_failure(
            &target,
            &expectation(&target),
            InitializationFailurePoint::AfterFirstCommit,
        );

        assert!(result.is_err());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
        assert!(target.exists());
    }

    #[test]
    fn initialization_never_overwrites_an_existing_file() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), "providers:\n  existing: true\n").unwrap();

        let discovery = initialize_target_configuration(&target, &expectation(&target)).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            fs::read_to_string(target.join("models.yml")).unwrap(),
            "providers:\n  existing: true\n"
        );
        assert_eq!(
            fs::read_to_string(target.join("config.yml")).unwrap(),
            "modelRoles: {}\n"
        );
    }

    #[test]
    fn changed_creation_plan_aborts_before_commit() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
        let expectation = expectation(&target);

        let result = initialize_target_configuration_with_hook(&target, &expectation, || {
            fs::remove_file(target.join("models.yml")).unwrap();
        });

        assert!(result.is_err());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
    }
    #[test]
    fn confirmation_token_rejects_same_path_replacement_directory() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let original = root.path().join("original-agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);
        fs::rename(&target, &original).unwrap();
        fs::create_dir(&target).unwrap();

        let error = initialize_target_configuration(&target, &expectation).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!target.join("models.yml").exists());
        assert!(!original.join("models.yml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn changed_symlink_target_aborts_initialization_without_partial_results() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_a = root.path().join("real-a");
        let real_b = root.path().join("real-b");
        let target = root.path().join("agent");
        fs::create_dir(&real_a).unwrap();
        fs::create_dir(&real_b).unwrap();
        symlink(&real_a, &target).unwrap();

        let expectation = expectation(&target);
        let result = initialize_target_configuration_with_hook(&target, &expectation, || {
            fs::remove_file(&target).unwrap();
            symlink(&real_b, &target).unwrap();
        });

        assert!(result.is_err());
        assert!(!real_a.join("models.yml").exists());
        assert!(!real_a.join("config.yml").exists());
        assert!(!real_b.join("models.yml").exists());
        assert!(!real_b.join("config.yml").exists());
    }

    #[cfg(unix)]
    #[test]
    fn changed_symlink_ancestor_aborts_missing_target_initialization() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_a = root.path().join("real-a");
        let real_b = root.path().join("real-b");
        let linked_parent = root.path().join("linked-parent");
        let target = linked_parent.join("nested").join("agent");
        fs::create_dir(&real_a).unwrap();
        fs::create_dir(&real_b).unwrap();
        symlink(&real_a, &linked_parent).unwrap();

        let expectation = expectation(&target);
        let result = initialize_target_configuration_with_hook(&target, &expectation, || {
            fs::remove_file(&linked_parent).unwrap();
            symlink(&real_b, &linked_parent).unwrap();
        });

        assert!(result.is_err());
        assert!(!real_a.join("nested").exists());
        assert!(!real_b.join("nested").exists());
    }

    #[cfg(unix)]
    #[test]
    fn confirmation_token_rejects_a_target_repointed_before_initialization() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_a = root.path().join("real-a");
        let real_b = root.path().join("real-b");
        let target = root.path().join("agent");
        fs::create_dir(&real_a).unwrap();
        fs::create_dir(&real_b).unwrap();
        symlink(&real_a, &target).unwrap();
        let expectation = expectation(&target);
        fs::remove_file(&target).unwrap();
        symlink(&real_b, &target).unwrap();

        let error = initialize_target_configuration(&target, &expectation).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);
        assert!(!real_a.join("models.yml").exists());
        assert!(!real_b.join("models.yml").exists());
    }
    #[cfg(unix)]
    #[test]
    fn crash_recovery_uses_the_original_resolved_parent_after_retarget() {
        use std::os::unix::fs::symlink;

        let root = tempdir().unwrap();
        let real_a = root.path().join("real-a");
        let real_b = root.path().join("real-b");
        let linked_parent = root.path().join("linked-parent");
        let target = linked_parent.join("agent");
        fs::create_dir(&real_a).unwrap();
        fs::create_dir(&real_b).unwrap();
        symlink(&real_a, &linked_parent).unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterFirstCommit,
        )
        .unwrap_err();
        assert!(real_a.join("agent/models.yml").exists());
        fs::remove_file(&linked_parent).unwrap();
        symlink(&real_b, &linked_parent).unwrap();

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(!real_a.join("agent").exists());
        assert!(!real_b.join("agent").exists());
    }

    #[cfg(unix)]
    #[test]
    fn publication_retarget_retains_manifest_and_reports_incomplete_recovery() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let moved_target = root.path().join("moved-agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_commit_hook(&target, &expectation, || {
            fs::rename(&target, &moved_target).unwrap();
            fs::create_dir(&target).unwrap();
        })
        .unwrap_err();

        assert!(error.recovery_incomplete());
        assert!(moved_target.join("models.yml").exists());
        assert!(moved_target.join("config.yml").exists());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_some()
        );
    }
    #[cfg(unix)]
    #[test]
    fn directory_publication_retarget_keeps_the_staged_handle_bound() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let moved_target = root.path().join("moved-agent");
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_directory_publish_hook(
            &target,
            &expectation,
            || {
                fs::rename(&target, &moved_target).unwrap();
                fs::create_dir(&target).unwrap();
                let marker = fs::read_dir(&moved_target)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.to_string_lossy().ends_with(".marker"))
                    .unwrap();
                fs::write(
                    target.join(marker.file_name().unwrap()),
                    fs::read(&marker).unwrap(),
                )
                .unwrap();
            },
        )
        .unwrap_err();

        assert!(error.recovery_incomplete(), "{error}");
        assert!(!moved_target.join("models.yml").exists());
        assert!(!moved_target.join("config.yml").exists());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
        assert_eq!(
            fs::read_dir(&target)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".marker"))
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn nested_directory_publish_failure_skips_path_rollback_after_swap() {
        let root = tempdir().unwrap();
        let missing_parent = root.path().join("nested");
        let target = missing_parent.join("agent");
        let moved_parent = root.path().join("moved-nested");
        let expectation = expectation(&target);
        let publish_count = std::cell::Cell::new(0);

        let error = initialize_target_configuration_with_directory_publish_hook(
            &target,
            &expectation,
            || {
                publish_count.set(publish_count.get() + 1);
                assert_eq!(publish_count.get(), 1);
                fs::rename(&missing_parent, &moved_parent).unwrap();
                fs::create_dir(&missing_parent).unwrap();
                let marker = fs::read_dir(&moved_parent)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| path.to_string_lossy().ends_with(".marker"))
                    .unwrap();
                fs::write(
                    missing_parent.join(marker.file_name().unwrap()),
                    fs::read(&marker).unwrap(),
                )
                .unwrap();
                fs::create_dir(moved_parent.join("agent")).unwrap();
                fs::write(moved_parent.join("agent/blocker"), "external\n").unwrap();
            },
        )
        .unwrap_err();

        assert!(error.recovery_incomplete(), "{error}");
        assert_eq!(publish_count.get(), 1);
        assert!(missing_parent.exists());
        assert_eq!(
            fs::read_dir(&missing_parent)
                .unwrap()
                .filter(|entry| entry
                    .as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".marker"))
                .count(),
            1
        );
        assert!(moved_parent.join("agent").exists());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_some()
        );
    }

    #[cfg(unix)]
    #[test]
    fn committed_cleanup_does_not_touch_a_replacement_directory() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let moved_target = root.path().join("moved-agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);

        let error =
            initialize_target_configuration_with_committed_hook(&target, &expectation, || {
                fs::rename(&target, &moved_target).unwrap();
                fs::create_dir(&target).unwrap();
                let staging_names: Vec<_> = fs::read_dir(&moved_target)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .filter(|name| {
                        let name = name.to_string_lossy();
                        name.starts_with(".omp-switch-init-") && name.ends_with(".tmp")
                    })
                    .collect();
                assert_eq!(staging_names.len(), 2);
                for name in staging_names {
                    fs::write(target.join(name), "attacker-owned\n").unwrap();
                }
            })
            .unwrap_err();

        assert!(error.recovery_incomplete());
        assert!(moved_target.join("models.yml").exists());
        assert!(moved_target.join("config.yml").exists());
        assert!(!fs::read_dir(&moved_target).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        let replacement_entries: Vec<_> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(replacement_entries.len(), 2);
        assert!(
            replacement_entries
                .iter()
                .all(|path| { fs::read_to_string(path).unwrap() == "attacker-owned\n" })
        );
        let transaction_root = test_transaction_root(&target).unwrap();
        let manifest_path = find_initialization_manifest(&target, &transaction_root)
            .unwrap()
            .expect("committed manifest must remain after identity loss");
        let manifest: InitializationManifest =
            serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.phase, InitializationPhase::Committed);
    }

    #[cfg(unix)]
    #[test]
    fn committed_cleanup_uses_created_directory_handle_after_retarget() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let moved_target = root.path().join("moved-agent");
        let expectation = expectation(&target);

        let error =
            initialize_target_configuration_with_committed_hook(&target, &expectation, || {
                fs::rename(&target, &moved_target).unwrap();
                fs::create_dir(&target).unwrap();
                let entries: Vec<_> = fs::read_dir(&moved_target)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name())
                    .collect();
                let marker_name = entries
                    .iter()
                    .find(|name| name.to_string_lossy().ends_with(".marker"))
                    .unwrap()
                    .clone();
                let marker_contents = fs::read(moved_target.join(&marker_name)).unwrap();
                fs::write(target.join(&marker_name), marker_contents).unwrap();
                let staging_names: Vec<_> = entries
                    .into_iter()
                    .filter(|name| name.to_string_lossy().ends_with(".tmp"))
                    .collect();
                assert_eq!(staging_names.len(), 2);
                for name in staging_names {
                    fs::write(target.join(name), "attacker-owned\n").unwrap();
                }
            })
            .unwrap_err();

        assert!(error.recovery_incomplete());
        assert!(moved_target.join("models.yml").exists());
        assert!(moved_target.join("config.yml").exists());
        assert!(!fs::read_dir(&moved_target).unwrap().any(|entry| {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            name.ends_with(".tmp") || name.ends_with(".marker")
        }));
        let replacement_entries: Vec<_> = fs::read_dir(&target)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        assert_eq!(replacement_entries.len(), 3);
        assert_eq!(
            replacement_entries
                .iter()
                .filter(|path| path.to_string_lossy().ends_with(".marker"))
                .count(),
            1
        );
        assert_eq!(
            replacement_entries
                .iter()
                .filter(|path| path.to_string_lossy().ends_with(".tmp"))
                .count(),
            2
        );
        assert!(
            replacement_entries
                .iter()
                .filter(|path| { path.to_string_lossy().ends_with(".tmp") })
                .all(|path| fs::read_to_string(path).unwrap() == "attacker-owned\n")
        );
        let transaction_root = test_transaction_root(&target).unwrap();
        let manifest_path = find_initialization_manifest(&target, &transaction_root)
            .unwrap()
            .expect("committed manifest must remain after target retarget");
        let manifest: InitializationManifest =
            serde_json::from_slice(&fs::read(manifest_path).unwrap()).unwrap();
        assert_eq!(manifest.phase, InitializationPhase::Committed);
    }

    #[test]
    fn crash_after_directory_marker_quarantines_unconfirmed_identity() {
        let root = tempdir().unwrap();
        let missing_parent = root.path().join("nested");
        let target = missing_parent.join("agent");
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterDirectoryMarker,
        )
        .unwrap_err();
        assert!(error.recovery_incomplete());
        assert!(!missing_parent.exists());
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));

        let recovery_error = discover_target_configuration(&target).unwrap_err();

        assert!(recovery_error.to_string().contains("身份尚未确认"));
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_none()
        );
        assert!(fs::read_dir(&transaction_root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".invalid-")
        }));
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn next_discovery_recovers_a_crash_before_directory_rename() {
        let root = tempdir().unwrap();
        let missing_parent = root.path().join("nested");
        let target = missing_parent.join("agent");
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashBeforeDirectoryRename,
        )
        .unwrap_err();
        assert!(error.recovery_incomplete());
        assert!(!missing_parent.exists());
        assert!(fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(recovered.recovery_notice.is_some());
        assert!(!missing_parent.exists());
        assert!(!fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
    }

    #[test]
    fn next_discovery_recovers_a_crash_after_directory_publication() {
        let root = tempdir().unwrap();
        let missing_parent = root.path().join("nested");
        let target = missing_parent.join("agent");
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterDirectoryCreation,
        )
        .unwrap_err();
        assert!(error.recovery_incomplete());
        assert!(target.exists());

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(recovered.recovery_notice.is_some());
        assert!(!missing_parent.exists());
    }

    #[test]
    fn next_discovery_recovers_a_crash_after_the_first_published_file() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);

        let error = initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterFirstCommit,
        )
        .unwrap_err();
        assert!(error.recovery_incomplete());
        assert!(target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
    }
    #[test]
    fn crash_recovery_refuses_a_same_path_replacement_directory() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let original = root.path().join("original-agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterFirstCommit,
        )
        .unwrap_err();
        fs::rename(&target, &original).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(
            target.join("models.yml"),
            "providers:\n  replacement: true\n",
        )
        .unwrap();

        let error = discover_target_configuration(&target).unwrap_err();

        assert!(error.to_string().contains("文件系统身份"));
        assert!(
            fs::read_to_string(target.join("models.yml"))
                .unwrap()
                .contains("replacement")
        );
        assert!(original.join("models.yml").exists());
    }

    #[test]
    fn prepared_recovery_preserves_an_externally_modified_published_file() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterFirstCommit,
        )
        .unwrap_err();
        fs::write(
            target.join("models.yml"),
            "providers:\n  externally-changed: true\n",
        )
        .unwrap();

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(
            recovered
                .recovery_notice
                .as_deref()
                .is_some_and(|notice| notice.contains("外部修改或替换"))
        );
        assert!(
            fs::read_to_string(target.join("models.yml"))
                .unwrap()
                .contains("externally-changed")
        );
        assert!(!target.join("config.yml").exists());
        assert!(!fs::read_dir(&target).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn committed_recovery_preserves_externally_modified_configuration() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterCommitMarker,
        )
        .unwrap_err();
        fs::write(
            target.join("models.yml"),
            "providers:\n  externally-changed: true\n",
        )
        .unwrap();

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(recovered.status, TargetConfigurationStatus::Writable);
        assert!(
            fs::read_to_string(target.join("models.yml"))
                .unwrap()
                .contains("externally-changed")
        );
        assert!(target.join("config.yml").exists());
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn committed_recovery_preserves_an_externally_deleted_file() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        fs::create_dir(&target).unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterCommitMarker,
        )
        .unwrap_err();
        fs::remove_file(target.join("config.yml")).unwrap();

        let recovered = discover_target_configuration(&target).unwrap();

        assert_eq!(
            recovered.status,
            TargetConfigurationStatus::CreationRequired
        );
        assert!(target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
        let transaction_root = test_transaction_root(&target).unwrap();
        assert!(
            find_initialization_manifest(&target, &transaction_root)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn tampered_manifest_is_quarantined_without_touching_injected_paths() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");
        let victim = root.path().join("victim.txt");
        fs::create_dir(&target).unwrap();
        fs::write(&victim, "do-not-touch").unwrap();
        let expectation = expectation(&target);
        initialize_target_configuration_with_failure(
            &target,
            &expectation,
            InitializationFailurePoint::CrashAfterCommitMarker,
        )
        .unwrap_err();
        let transaction_root = test_transaction_root(&target).unwrap();
        let manifest_path = find_initialization_manifest(&target, &transaction_root)
            .unwrap()
            .expect("simulated crash must leave a manifest");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest["entries"][0]["destination"] =
            serde_json::Value::String(victim.to_string_lossy().into_owned());
        fs::write(&manifest_path, serde_json::to_vec(&manifest).unwrap()).unwrap();

        let error = discover_target_configuration(&target).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read_to_string(&victim).unwrap(), "do-not-touch");
        assert!(!manifest_path.exists());
        assert!(fs::read_dir(&transaction_root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("transaction-")
        }));
    }
}
