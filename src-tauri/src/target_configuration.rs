use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
static PROBE_SEQUENCE: AtomicU64 = AtomicU64::new(0);

use serde::Serialize;

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
    pub create_paths: Vec<String>,
    pub warnings: Vec<String>,
    pub issue: Option<ConfigurationIssue>,
}

pub fn discover_target_configuration(
    target: &Path,
) -> std::io::Result<TargetConfigurationDiscovery> {
    let target_metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => Some(metadata),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let target_exists = target_metadata.is_some();
    let resolved_path = if target_exists {
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
    if target_exists && !fs::metadata(target)?.is_dir() {
        return Ok(unsafe_discovery(
            target,
            "Target configuration 路径不是目录。".to_owned(),
        ));
    }
    let (models, models_issue) = discover_file(target, "models", &["models.json"])?;
    let (config, config_issue) =
        discover_file(target, "config", &["settings.json", "config.json"])?;
    let access_root = if target_exists {
        target
    } else {
        nearest_existing_ancestor(target)?
    };
    let directory_writable = probe_directory_write(access_root)?;
    let status = combined_status(&models.status, &config.status, directory_writable);
    let writable = status == TargetConfigurationStatus::Writable;
    let create_paths = creation_paths(target, target_exists, &models.status, &config.status)?;
    let mut warnings = Vec::new();
    for name in ["models", "config"] {
        if target.join(format!("{name}.yml")).exists()
            && target.join(format!("{name}.yaml")).exists()
        {
            warnings.push(format!(
                "检测到 {name}.yaml；OMP Switch 使用 {name}.yml，且 {name}.yaml 不会被修改。"
            ));
        }
    }
    Ok(TargetConfigurationDiscovery {
        path: target.to_string_lossy().into_owned(),
        resolved_path,
        status,
        writable,
        models,
        config,
        create_paths,
        warnings,
        issue: models_issue.or(config_issue),
    })
}

fn discover_file(
    target: &Path,
    stem: &str,
    legacy_names: &[&str],
) -> std::io::Result<(ConfigurationFileDiscovery, Option<ConfigurationIssue>)> {
    let canonical_path = target.join(format!("{stem}.yml"));
    let alternate_path = target.join(format!("{stem}.yaml"));
    let (mut status, selected_path) = if path_entry_exists(&canonical_path)? {
        let status = if path_entry_exists(&alternate_path)? {
            ConfigurationFileStatus::CanonicalWithAlternate
        } else {
            ConfigurationFileStatus::Normal
        };
        (status, Some(canonical_path.as_path()))
    } else if path_entry_exists(&alternate_path)? {
        (
            ConfigurationFileStatus::AlternateOnly,
            Some(alternate_path.as_path()),
        )
    } else if legacy_names.iter().any(|name| target.join(name).exists()) {
        (ConfigurationFileStatus::LegacyJson, None)
    } else {
        (ConfigurationFileStatus::Missing, None)
    };
    let (resolved_path, resolution_issue) = if let Some(path) = selected_path {
        match path.canonicalize() {
            Ok(resolved) => (Some(resolved.to_string_lossy().into_owned()), None),
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
    let parse_issue = if resolution_issue.is_none() {
        if let Some(path) = selected_path {
            match fs::read_to_string(path) {
                Ok(contents) => match serde_yaml::from_str::<serde_yaml::Value>(&contents) {
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
                Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
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
        && selected_path.is_some_and(|path| !file_is_writable(path))
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

fn nearest_existing_ancestor(path: &Path) -> std::io::Result<&Path> {
    let mut candidate = path;
    loop {
        if candidate.try_exists()? {
            if fs::metadata(candidate)?.is_dir() {
                return Ok(candidate);
            }
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "target ancestor is not a directory",
            ));
        }
        candidate = candidate.parent().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "target has no existing ancestor",
            )
        })?;
    }
}

fn file_is_writable(path: &Path) -> bool {
    OpenOptions::new().read(true).write(true).open(path).is_ok()
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
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => Ok(false),
        Err(error) => Err(error),
    }
}

fn creation_paths(
    target: &Path,
    target_exists: bool,
    models: &ConfigurationFileStatus,
    config: &ConfigurationFileStatus,
) -> io::Result<Vec<String>> {
    let mut paths = Vec::new();
    if !target_exists {
        paths.extend(
            missing_directories(target)?
                .into_iter()
                .map(|path| path.to_string_lossy().into_owned()),
        );
    }
    if matches!(models, ConfigurationFileStatus::Missing) {
        paths.push(target.join("models.yml").to_string_lossy().into_owned());
    }
    if matches!(config, ConfigurationFileStatus::Missing) {
        paths.push(target.join("config.yml").to_string_lossy().into_owned());
    }
    Ok(paths)
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
        create_paths: Vec::new(),
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
    } else if statuses.iter().any(|status| {
        matches!(
            status,
            ConfigurationFileStatus::AlternateOnly | ConfigurationFileStatus::ReadOnly
        )
    }) || !directory_writable
    {
        TargetConfigurationStatus::ReadOnly
    } else if statuses
        .iter()
        .any(|status| matches!(status, ConfigurationFileStatus::Missing))
    {
        TargetConfigurationStatus::CreationRequired
    } else {
        TargetConfigurationStatus::Writable
    }
}

const MINIMAL_MODELS_YAML: &str = "providers: {}\n";
const MINIMAL_CONFIG_YAML: &str = "modelRoles: {}\n";
static INITIALIZATION_SEQUENCE: AtomicU64 = AtomicU64::new(0);

pub fn initialize_target_configuration(
    target: &Path,
    expected_create_paths: &[String],
) -> std::io::Result<TargetConfigurationDiscovery> {
    initialize_target_configuration_internal(target, expected_create_paths, false, false, || {})
}

#[cfg(test)]
#[derive(Clone, Copy)]
pub(crate) enum InitializationFailurePoint {
    CorruptConfigBeforeValidation,
    AfterFirstCommit,
}

#[cfg(test)]
pub(crate) fn initialize_target_configuration_with_failure(
    target: &Path,
    expected_create_paths: &[String],
    failure: InitializationFailurePoint,
) -> std::io::Result<TargetConfigurationDiscovery> {
    initialize_target_configuration_internal(
        target,
        expected_create_paths,
        matches!(
            failure,
            InitializationFailurePoint::CorruptConfigBeforeValidation
        ),
        matches!(failure, InitializationFailurePoint::AfterFirstCommit),
        || {},
    )
}

#[cfg(test)]
fn initialize_target_configuration_with_hook(
    target: &Path,
    expected_create_paths: &[String],
    before_prepare: impl FnOnce(),
) -> std::io::Result<TargetConfigurationDiscovery> {
    initialize_target_configuration_internal(
        target,
        expected_create_paths,
        false,
        false,
        before_prepare,
    )
}

fn initialize_target_configuration_internal(
    target: &Path,
    expected_create_paths: &[String],
    corrupt_config_before_validation: bool,
    fail_after_first_commit: bool,
    before_prepare: impl FnOnce(),
) -> std::io::Result<TargetConfigurationDiscovery> {
    let initial = discover_target_configuration(target)?;
    if initial.status == TargetConfigurationStatus::Writable && expected_create_paths.is_empty() {
        return Ok(initial);
    }
    ensure_creation_plan_unchanged(&initial, expected_create_paths)?;
    let ancestor = snapshot_existing_ancestor(target)?;
    let mut created_directories = Vec::new();
    let mut temporary_files = Vec::new();
    let mut created_files = Vec::new();
    before_prepare();
    let operation = (|| {
        let current_ancestor = ancestor.path.canonicalize()?;
        if current_ancestor != ancestor.resolved {
            return Err(io::Error::other(
                "Target configuration 现有父路径在初始化期间发生变化",
            ));
        }
        create_missing_directories(target, &mut created_directories)?;
        let real_target = target.canonicalize()?;
        if real_target != ancestor.expected_target {
            return Err(io::Error::other(
                "Target configuration 链接目标在初始化期间发生变化",
            ));
        }
        let expected_after_directories: Vec<String> = expected_create_paths
            .iter()
            .filter(|path| {
                !created_directories
                    .iter()
                    .any(|directory| Path::new(path.as_str()) == directory)
            })
            .cloned()
            .collect();
        let after_directory_creation = discover_target_configuration(target)?;
        ensure_creation_plan_unchanged(&after_directory_creation, &expected_after_directories)?;

        let mut pending = Vec::new();
        for (name, contents, missing) in [
            (
                "models.yml",
                MINIMAL_MODELS_YAML,
                after_directory_creation.models.status == ConfigurationFileStatus::Missing,
            ),
            (
                "config.yml",
                MINIMAL_CONFIG_YAML,
                after_directory_creation.config.status == ConfigurationFileStatus::Missing,
            ),
        ] {
            if !missing {
                continue;
            }
            let sequence = INITIALIZATION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let temporary = real_target.join(format!(
                ".omp-switch-init-{}-{sequence}-{name}.tmp",
                std::process::id()
            ));
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temporary)?;
            let bytes = if corrupt_config_before_validation && name == "config.yml" {
                b"modelRoles: [\n".as_slice()
            } else {
                contents.as_bytes()
            };
            file.write_all(bytes)?;
            file.sync_all()?;
            temporary_files.push(temporary.clone());
            let serialized = fs::read_to_string(&temporary)?;
            serde_yaml::from_str::<serde_yaml::Value>(&serialized).map_err(|error| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("最小配置验证失败：{error}"),
                )
            })?;
            pending.push((temporary, real_target.join(name)));
        }

        let before_commit = discover_target_configuration(target)?;
        ensure_creation_plan_unchanged(&before_commit, &expected_after_directories)?;

        for (index, (temporary, destination)) in pending.into_iter().enumerate() {
            fs::hard_link(&temporary, &destination)?;
            created_files.push(destination);
            fs::remove_file(&temporary)?;
            temporary_files.retain(|path| path != &temporary);
            if fail_after_first_commit && index == 0 {
                return Err(io::Error::other("injected initialization commit failure"));
            }
        }

        let final_discovery = discover_target_configuration(target)?;
        if final_discovery.status != TargetConfigurationStatus::Writable {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "创建后的 Target configuration 未通过重新发现和解析",
            ));
        }
        Ok(final_discovery)
    })();

    match operation {
        Ok(discovery) => Ok(discovery),
        Err(error) => {
            match rollback_initialization(&temporary_files, &created_files, &created_directories) {
                Ok(()) => Err(error),
                Err(rollback_error) => Err(io::Error::new(
                    error.kind(),
                    format!("{error}；{rollback_error}"),
                )),
            }
        }
    }
}

fn ensure_creation_plan_unchanged(
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
            "Target configuration 在确认后发生变化",
        ))
    }
}

struct ExistingAncestorSnapshot {
    path: PathBuf,
    resolved: PathBuf,
    expected_target: PathBuf,
}

fn snapshot_existing_ancestor(target: &Path) -> io::Result<ExistingAncestorSnapshot> {
    let mut candidate = target;
    loop {
        match fs::symlink_metadata(candidate) {
            Ok(_) => {
                let resolved = candidate.canonicalize()?;
                let suffix = target.strip_prefix(candidate).map_err(io::Error::other)?;
                let expected_target = resolved.join(suffix);
                return Ok(ExistingAncestorSnapshot {
                    path: candidate.to_path_buf(),
                    resolved,
                    expected_target,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
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

fn missing_directories(target: &Path) -> io::Result<Vec<PathBuf>> {
    let mut missing = Vec::new();
    let mut candidate = target;
    loop {
        match fs::symlink_metadata(candidate) {
            Ok(_) => break,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(candidate.to_path_buf());
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
    missing.reverse();
    Ok(missing)
}

fn create_missing_directories(target: &Path, created: &mut Vec<PathBuf>) -> io::Result<()> {
    for directory in missing_directories(target)? {
        fs::create_dir(&directory)?;
        created.push(directory);
    }
    Ok(())
}

fn rollback_initialization(
    temporary_files: &[PathBuf],
    created_files: &[PathBuf],
    created_directories: &[PathBuf],
) -> io::Result<()> {
    let mut failures = Vec::new();
    for path in temporary_files
        .iter()
        .rev()
        .chain(created_files.iter().rev())
    {
        if let Err(error) = fs::remove_file(path)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    for path in created_directories.iter().rev() {
        if let Err(error) = fs::remove_dir(path)
            && error.kind() != io::ErrorKind::NotFound
        {
            failures.push(format!("{}: {error}", path.display()));
        }
    }
    if failures.is_empty() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "回滚失败：{}",
            failures.join("；")
        )))
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::{
        ConfigurationFileStatus, InitializationFailurePoint, TargetConfigurationStatus,
        discover_target_configuration, initialize_target_configuration,
        initialize_target_configuration_with_failure, initialize_target_configuration_with_hook,
        rollback_initialization,
    };

    fn creation_paths(target: &Path) -> Vec<String> {
        discover_target_configuration(target).unwrap().create_paths
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
        let command = format!(
            "mklink /J \"{}\" \"{}\"",
            junction.display(),
            real.display()
        );
        assert!(
            Command::new("cmd")
                .args(["/C", &command])
                .status()
                .unwrap()
                .success()
        );

        let discovery = discover_target_configuration(&junction).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            discovery.resolved_path.as_deref(),
            Some(real.to_string_lossy().as_ref())
        );
    }

    #[test]
    fn atomically_creates_and_reparses_minimal_configuration() {
        let root = tempdir().unwrap();
        let target = root.path().join("nested").join("agent");

        let discovery = initialize_target_configuration(&target, &creation_paths(&target)).unwrap();

        assert_eq!(discovery.status, TargetConfigurationStatus::Writable);
        assert_eq!(
            fs::read(target.join("models.yml")).unwrap(),
            b"providers: {}\n"
        );
        assert_eq!(
            fs::read(target.join("config.yml")).unwrap(),
            b"modelRoles: {}\n"
        );
    }

    #[test]
    fn validation_failure_leaves_no_partial_directory_or_files() {
        let root = tempdir().unwrap();
        let target = root.path().join("agent");

        let result = initialize_target_configuration_with_failure(
            &target,
            &creation_paths(&target),
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
            &creation_paths(&target),
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

        let discovery = initialize_target_configuration(&target, &creation_paths(&target)).unwrap();

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
        let expected_create_paths = creation_paths(&target);

        let result =
            initialize_target_configuration_with_hook(&target, &expected_create_paths, || {
                fs::remove_file(target.join("models.yml")).unwrap();
            });

        assert!(result.is_err());
        assert!(!target.join("models.yml").exists());
        assert!(!target.join("config.yml").exists());
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

        let expected_create_paths = creation_paths(&target);
        let result =
            initialize_target_configuration_with_hook(&target, &expected_create_paths, || {
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

        let expected_create_paths = creation_paths(&target);
        let result =
            initialize_target_configuration_with_hook(&target, &expected_create_paths, || {
                fs::remove_file(&linked_parent).unwrap();
                symlink(&real_b, &linked_parent).unwrap();
            });

        assert!(result.is_err());
        assert!(!real_a.join("nested").exists());
        assert!(!real_b.join("nested").exists());
    }

    #[test]
    fn rollback_reports_paths_it_cannot_remove() {
        let root = tempdir().unwrap();
        let created_directory = root.path().join("created");
        fs::create_dir(&created_directory).unwrap();
        fs::write(created_directory.join("external-file"), "keep").unwrap();

        let error = rollback_initialization(&[], &[], std::slice::from_ref(&created_directory))
            .unwrap_err();

        assert!(error.to_string().contains("回滚失败"));
        assert!(
            error
                .to_string()
                .contains(created_directory.to_string_lossy().as_ref())
        );
    }
}
