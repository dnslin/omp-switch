use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use atomic_write_file::AtomicWriteFile;
use fs4::{FileExt, TryLockError};
use serde_yaml::Value;
use sha2::{Digest, Sha256};

use crate::{
    error::{AppError, io_error_cause},
    redaction::redact_diagnostic,
    target_configuration::{
        ConfigurationFileStatus, TargetConfigurationDiscovery, TargetConfigurationStatus,
    },
};

const BACKUP_ALLOCATION_ATTEMPTS: usize = 128;
const MAX_BACKUPS_PER_FILE: usize = 10;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ModelsWriteFailurePoint {
    BeforeBackup,
    AfterBackup,
    BeforeTemporaryWrite,
    CorruptTemporaryFile,
    MutateUntouchedValue,
    BeforeReplacement,
    #[cfg(all(test, unix))]
    CommitFailure,
    #[cfg(test)]
    AfterAtomicReplacement,
    #[cfg(all(test, unix))]
    BackupFilePermissionAndCleanupFailure,
    #[cfg(test)]
    BackupDirectoryCreationFailure,
    #[cfg(test)]
    BackupFileOpenFailure,
    #[cfg(test)]
    BackupFileWriteFailure,
    #[cfg(test)]
    BackupFileSyncFailure,
    #[cfg(test)]
    TemporaryFileOpenFailure,
    #[cfg(test)]
    TemporaryFileWriteFailure,
    #[cfg(test)]
    TemporaryFileSyncFailure,
    #[cfg(test)]
    MutateConfigBeforeReplacement,
}
pub(crate) trait ModelsMutation {
    fn verb(&self) -> &'static str;
    fn serialization_error(&self) -> (&'static str, &'static str, &'static str);
    fn apply(&self, tree: &mut Value) -> Result<(), AppError>;
    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError>;
    fn validate_before_commit(&self, _loaded: &LoadedModels) -> Result<(), AppError> {
        Ok(())
    }
    #[cfg(test)]
    fn mutate_external_state_for_test(&self) {}
}
pub(crate) struct LoadedModels {
    pub(crate) expected_target: PathBuf,
    pub(crate) models_path: PathBuf,
    pub(crate) backup_partition: &'static str,
    pub(crate) original_bytes: Vec<u8>,
    pub(crate) original_hash: String,
    pub(crate) original_tree: Value,
}
pub(crate) fn load_models_for_write(
    target: &TargetConfigurationDiscovery,
    opened_models_hash: &str,
) -> Result<LoadedModels, AppError> {
    validate_writable_target(target)?;
    let expected_target = resolved_path(&target.resolved_path, "Target configuration")?;
    let models_path = resolved_path(&target.models.resolved_path, "models.yml")?;
    ensure_resolved_models_path(&models_path, &expected_target)?;

    let original_bytes =
        fs::read(&models_path).map_err(|error| write_error("read_models", error))?;
    let original_hash = content_hash(&original_bytes);
    if original_hash != opened_models_hash {
        return Err(hash_conflict());
    }
    let original_tree = serde_yaml::from_slice::<Value>(&original_bytes).map_err(|error| {
        yaml_error(
            "parse_original_models",
            "models-parse-error",
            "models.yml 已无法重新解析",
            "请在外部修复 YAML 后重新读取；OMP Switch 不会覆盖该文件。",
            error,
        )
    })?;
    Ok(LoadedModels {
        expected_target,
        models_path,
        backup_partition: "models",
        original_bytes,
        original_hash,
        original_tree,
    })
}
pub(crate) fn load_config_for_write(
    target: &TargetConfigurationDiscovery,
    opened_config_hash: &str,
) -> Result<LoadedModels, AppError> {
    let mut config_target = target.clone();
    config_target.models = target.config.clone();
    let mut loaded = load_models_for_write(&config_target, opened_config_hash)?;
    loaded.backup_partition = "config";
    Ok(loaded)
}

pub(crate) fn write_models_mutation<M: ModelsMutation>(
    backup_root: &Path,
    loaded: &LoadedModels,
    mutation: &M,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<(), AppError> {
    let verb = mutation.verb();
    if failure == Some(ModelsWriteFailurePoint::BeforeBackup) {
        return Err(injected_failure(format!("{verb}当前备份前发生故障。")));
    }
    let _write_lock = acquire_models_write_lock(backup_root, &loaded.expected_target)?;
    ensure_no_pending_configuration_transaction(backup_root, &loaded.expected_target)?;
    create_current_backup(
        backup_root,
        &loaded.expected_target,
        loaded.backup_partition,
        &loaded.original_bytes,
        failure,
    )?;
    if failure == Some(ModelsWriteFailurePoint::AfterBackup) {
        return Err(injected_failure(format!("{verb}当前备份后发生故障。")));
    }

    let mut candidate = loaded.original_tree.clone();
    mutation.apply(&mut candidate)?;
    let (serialize_operation, serialize_message, serialize_action) = mutation.serialization_error();
    let serialized = serde_yaml::to_string(&candidate).map_err(|error| {
        yaml_error(
            serialize_operation,
            "models-serialize-error",
            serialize_message,
            serialize_action,
            error,
        )
    })?;

    if failure == Some(ModelsWriteFailurePoint::BeforeTemporaryWrite) {
        return Err(injected_failure(format!("{verb}临时文件前发生故障。")));
    }
    #[cfg(test)]
    inject_io_failure(failure, ModelsWriteFailurePoint::TemporaryFileOpenFailure)
        .map_err(|error| write_error("open_models_temporary", error))?;
    let mut temporary = AtomicWriteFile::options()
        .read(true)
        .open(&loaded.models_path)
        .map_err(|error| write_error("open_models_temporary", error))?;
    #[cfg(test)]
    inject_io_failure(failure, ModelsWriteFailurePoint::TemporaryFileWriteFailure)
        .map_err(|error| write_error("write_models_temporary", error))?;
    temporary
        .write_all(serialized.as_bytes())
        .map_err(|error| write_error("write_models_temporary", error))?;
    #[cfg(test)]
    inject_io_failure(failure, ModelsWriteFailurePoint::TemporaryFileSyncFailure)
        .map_err(|error| write_error("write_models_temporary", error))?;
    temporary
        .sync_all()
        .map_err(|error| write_error("write_models_temporary", error))?;
    if failure == Some(ModelsWriteFailurePoint::CorruptTemporaryFile) {
        temporary
            .set_len(0)
            .and_then(|()| temporary.seek(SeekFrom::Start(0)).map(|_| ()))
            .and_then(|()| temporary.write_all(b"providers: [\n"))
            .and_then(|()| temporary.sync_all())
            .map_err(|error| write_error("corrupt_models_temporary", error))?;
    }
    temporary
        .seek(SeekFrom::Start(0))
        .map_err(|error| write_error("rewind_models_temporary", error))?;
    let mut temporary_bytes = Vec::new();
    temporary
        .read_to_end(&mut temporary_bytes)
        .map_err(|error| write_error("read_models_temporary", error))?;
    let mut reparsed = serde_yaml::from_slice::<Value>(&temporary_bytes).map_err(|error| {
        yaml_error(
            "parse_temporary_models",
            "models-temporary-parse-error",
            "临时 models.yml 无法重新解析；原文件没有被修改",
            "请检查表单后重试。",
            error,
        )
    })?;
    if failure == Some(ModelsWriteFailurePoint::MutateUntouchedValue) {
        mutate_untouched_value_for_test(&mut reparsed);
    }
    mutation.validate(&reparsed, &loaded.original_tree)?;

    ensure_resolved_models_path(&loaded.models_path, &loaded.expected_target)?;
    let latest_bytes =
        fs::read(&loaded.models_path).map_err(|error| write_error("recheck_models", error))?;
    if content_hash(&latest_bytes) != loaded.original_hash {
        return Err(hash_conflict());
    }
    #[cfg(test)]
    if failure == Some(ModelsWriteFailurePoint::MutateConfigBeforeReplacement) {
        mutation.mutate_external_state_for_test();
    }
    mutation.validate_before_commit(loaded)?;
    if failure == Some(ModelsWriteFailurePoint::BeforeReplacement) {
        return Err(injected_failure(format!("{verb}原子替换前发生故障。")));
    }
    #[cfg(all(test, unix))]
    let restricted_directory =
        restrict_models_directory_for_commit_failure(&loaded.models_path, failure)?;
    let commit_result = temporary.commit();
    #[cfg(all(test, unix))]
    restore_models_directory_permissions_for_test(restricted_directory)?;
    #[cfg(test)]
    let commit_result: std::io::Result<()> =
        if failure == Some(ModelsWriteFailurePoint::AfterAtomicReplacement) {
            Err(std::io::Error::other(
                "injected error after the atomic replacement",
            ))
        } else {
            commit_result
        };
    if let Err(error) = commit_result {
        match fs::read(&loaded.models_path) {
            Ok(bytes) if bytes.as_slice() == serialized.as_bytes() => {
                tracing::warn!(
                    operation = "reconcile_models_replacement",
                    cause = io_error_cause(error.kind()),
                    status = "committed",
                    "Atomic models replacement reported an error after commit"
                );
            }
            Ok(bytes) if content_hash(&bytes) == loaded.original_hash => {
                return Err(write_error("replace_models", error));
            }
            Ok(_) => return Err(replacement_outcome_unknown(error, None)),
            Err(observation_error) => {
                return Err(replacement_outcome_unknown(
                    error,
                    Some(observation_error.kind()),
                ));
            }
        }
    }

    Ok(())
}

#[cfg(all(test, unix))]
fn restrict_models_directory_for_commit_failure(
    models_path: &Path,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<Option<(PathBuf, fs::Permissions)>, AppError> {
    if failure != Some(ModelsWriteFailurePoint::CommitFailure) {
        return Ok(None);
    }
    let directory = models_path
        .parent()
        .ok_or_else(|| AppError::internal("models.yml 缺少用于原子替换的父目录。"))?;
    let original_permissions = fs::metadata(directory)
        .map_err(|error| write_error("read_models_directory_permissions", error))?
        .permissions();
    let mut restricted_permissions = original_permissions.clone();
    restricted_permissions.set_mode(original_permissions.mode() & !0o222);
    fs::set_permissions(directory, restricted_permissions)
        .map_err(|error| write_error("restrict_models_directory_permissions", error))?;
    Ok(Some((directory.to_owned(), original_permissions)))
}

#[cfg(all(test, unix))]
fn restore_models_directory_permissions_for_test(
    restricted_directory: Option<(PathBuf, fs::Permissions)>,
) -> Result<(), AppError> {
    if let Some((directory, permissions)) = restricted_directory {
        fs::set_permissions(directory, permissions)
            .map_err(|error| write_error("restore_models_directory_permissions", error))?;
    }
    Ok(())
}
pub(crate) fn ensure_no_pending_configuration_transaction(
    backup_root: &Path,
    resolved_target: &Path,
) -> Result<(), AppError> {
    let directory = backup_root
        .join(target_fingerprint(resolved_target))
        .join("transactions");
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            tracing::warn!(
                operation = "inspect_pending_configuration_transaction",
                cause = io_error_cause(error.kind()),
                "Configuration transaction directory could not be inspected"
            );
            return Err(pending_configuration_transaction_error());
        }
    };
    for entry in entries {
        let path = entry
            .map_err(|error| {
                tracing::warn!(
                    operation = "inspect_pending_configuration_transaction_entry",
                    cause = io_error_cause(error.kind()),
                    "Configuration transaction entry could not be inspected"
                );
                pending_configuration_transaction_error()
            })?
            .path();
        if path.extension().and_then(|value| value.to_str()) == Some("json") {
            return Err(pending_configuration_transaction_error());
        }
    }
    Ok(())
}

fn pending_configuration_transaction_error() -> AppError {
    AppError::new(
        "configuration-transaction-pending",
        "Target configuration 存在尚未完成的 Configuration transaction。",
        "请重新检测 OMP，先完成事务清理或整体恢复后再写入；当前操作没有修改配置文件。",
    )
}

pub(crate) fn acquire_configuration_lock(
    backup_root: &Path,
    resolved_target: &Path,
) -> Result<fs::File, AppError> {
    acquire_lock(backup_root, resolved_target, "target.lock")
}

pub(crate) fn acquire_configuration_file_lock(
    backup_root: &Path,
    resolved_target: &Path,
    resolved_file: &Path,
) -> Result<fs::File, AppError> {
    let lock_name = format!("{}.lock", target_fingerprint(resolved_file));
    acquire_lock(backup_root, resolved_target, &lock_name)
}

fn acquire_lock(
    backup_root: &Path,
    resolved_target: &Path,
    lock_name: &str,
) -> Result<fs::File, AppError> {
    let target_id = target_fingerprint(resolved_target);
    let lock_directory = backup_root
        .join(target_id)
        .join("transactions")
        .join("locks");
    create_private_backup_directory(&lock_directory, None)
        .map_err(|error| write_error("create_configuration_lock_directory", error))?;
    let lock_path = lock_directory.join(lock_name);
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options
        .open(&lock_path)
        .map_err(|error| write_error("open_configuration_write_lock", error))?;
    set_private_backup_file_permissions(&lock_path, None)
        .map_err(|error| write_error("secure_configuration_write_lock", error))?;
    match FileExt::try_lock(&lock) {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(models_write_in_progress()),
        Err(TryLockError::Error(error)) => Err(write_error("lock_configuration_write", error)),
    }
}

fn acquire_models_write_lock(
    backup_root: &Path,
    resolved_target: &Path,
) -> Result<fs::File, AppError> {
    acquire_configuration_lock(backup_root, resolved_target)
}

fn models_write_in_progress() -> AppError {
    AppError::new(
        "models-write-in-progress",
        "另一项 OMP Switch 配置写入正在进行。",
        "请等待当前写入完成后重新读取配置，再检查表单并重试。",
    )
}

fn yaml_error(
    operation: &'static str,
    code: &'static str,
    message: &'static str,
    action: &'static str,
    error: serde_yaml::Error,
) -> AppError {
    let diagnostic = redact_diagnostic(&error.to_string());
    tracing::warn!(operation, diagnostic = %diagnostic, "Provider creation YAML operation failed");
    AppError::new(code, format!("{message}：{diagnostic}"), action)
}

pub(crate) fn validate_writable_target(
    target: &TargetConfigurationDiscovery,
) -> Result<(), AppError> {
    if target.status != TargetConfigurationStatus::Writable || !target.writable {
        return Err(AppError::new(
            "provider-create-unavailable",
            "当前 Target configuration 不允许创建 Provider。",
            "请重新检测 OMP，并按当前只读、迁移或错误提示处理。",
        ));
    }
    if !matches!(
        target.models.status,
        ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
    ) {
        return Err(AppError::new(
            "provider-create-unavailable",
            "当前 models.yml 不允许安全写入。",
            "请重新读取配置并处理 models.yml 的当前状态。",
        ));
    }
    Ok(())
}

pub(crate) fn resolved_path(path: &Option<String>, label: &str) -> Result<PathBuf, AppError> {
    path.as_deref().map(PathBuf::from).ok_or_else(|| {
        AppError::new(
            "provider-create-unavailable",
            format!("无法确认 {label} 的真实路径。"),
            "请重新检测 OMP 并修复链接、路径类型或权限问题。",
        )
    })
}

pub(crate) fn ensure_resolved_models_path(
    models_path: &Path,
    expected_target: &Path,
) -> Result<(), AppError> {
    ensure_resolved_file_path(models_path, expected_target, "models.yml")
}

pub(crate) fn ensure_resolved_file_path(
    file_path: &Path,
    expected_target: &Path,
    label: &str,
) -> Result<(), AppError> {
    let resolved_target = expected_target
        .canonicalize()
        .map_err(|error| write_error("resolve_target", error))?;
    let resolved_file = file_path
        .canonicalize()
        .map_err(|error| write_error("resolve_configuration_file", error))?;
    if resolved_target != expected_target || resolved_file != file_path {
        return Err(AppError::new(
            "provider-create-target-changed",
            format!("Target configuration 的 {label} 真实文件目标已变化。"),
            "请重新检测 OMP；OMP Switch 不会向变化后的路径写入。",
        ));
    }
    if !fs::metadata(&resolved_file)
        .map_err(|error| write_error("inspect_configuration_file", error))?
        .is_file()
    {
        return Err(AppError::new(
            "provider-create-target-changed",
            format!("{label} 的真实目标不是普通文件。"),
            "请修复路径后重新检测 OMP。",
        ));
    }
    Ok(())
}
fn backup_partition(partition: &str) -> Result<&str, AppError> {
    match partition {
        "models" | "config" => Ok(partition),
        _ => Err(AppError::internal("Configuration transaction 备份分区无效")),
    }
}
fn create_current_backup(
    backup_root: &Path,
    target_configuration: &Path,
    partition: &str,
    original_bytes: &[u8],
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<(), AppError> {
    let target_directory = backup_root.join(target_fingerprint(target_configuration));
    let directory = target_directory.join(backup_partition(partition)?);
    for path in [backup_root, target_directory.as_path(), directory.as_path()] {
        create_private_backup_directory(path, failure)
            .map_err(|error| write_error("create_backup_directory", error))?;
    }
    let (backup_path, mut backup) = allocate_backup_file(&directory, failure)
        .map_err(|error| write_error("create_models_backup", error))?;
    let result = (|| -> std::io::Result<()> {
        #[cfg(test)]
        inject_io_failure(failure, ModelsWriteFailurePoint::BackupFileWriteFailure)?;
        backup.write_all(original_bytes)?;
        #[cfg(test)]
        inject_io_failure(failure, ModelsWriteFailurePoint::BackupFileSyncFailure)?;
        backup.sync_all()?;
        drop(backup);
        if fs::read(&backup_path)? != original_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "backup contents differ from models.yml",
            ));
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            if let Err(error) = prune_backup_files(&directory, &backup_path) {
                tracing::warn!(
                    operation = "prune_configuration_backups",
                    cause = io_error_cause(error.kind()),
                    "Configuration backup retention cleanup failed"
                );
            }
            Ok(())
        }
        Err(error) => {
            cleanup_partial_backup(&backup_path);
            Err(write_error("create_models_backup", error))
        }
    }
}
pub(crate) fn create_transaction_backup(
    backup_root: &Path,
    target_configuration: &Path,
    original_bytes: &[u8],
    partition: &str,
    transaction_id: &str,
) -> Result<PathBuf, AppError> {
    if transaction_id.is_empty()
        || transaction_id.chars().any(|character| {
            !character.is_ascii_alphanumeric() && character != '-' && character != '_'
        })
    {
        return Err(AppError::internal("Configuration transaction ID 无效"));
    }
    let target_directory = backup_root.join(target_fingerprint(target_configuration));
    let directory = target_directory.join(backup_partition(partition)?);
    for path in [backup_root, target_directory.as_path(), directory.as_path()] {
        create_private_backup_directory(path, None)
            .map_err(|error| write_error("create_transaction_backup_directory", error))?;
    }
    let backup_path = directory.join(format!("tx-{transaction_id}.yml"));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut backup = options
        .open(&backup_path)
        .map_err(|error| write_error("open_transaction_backup", error))?;
    set_private_backup_file_permissions(&backup_path, None)
        .map_err(|error| write_error("secure_transaction_backup", error))?;
    let result = (|| -> std::io::Result<()> {
        backup.write_all(original_bytes)?;
        backup.sync_all()?;
        drop(backup);
        if fs::read(&backup_path)? != original_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "transaction backup contents differ from target",
            ));
        }
        Ok(())
    })();
    match result {
        Ok(()) => {
            if let Err(error) = prune_backup_files(&directory, &backup_path) {
                tracing::warn!(
                    operation = "prune_configuration_backups",
                    cause = io_error_cause(error.kind()),
                    "Configuration backup retention cleanup failed"
                );
            }
            Ok(backup_path)
        }
        Err(error) => {
            cleanup_partial_backup(&backup_path);
            Err(write_error("create_transaction_backup", error))
        }
    }
}

fn prune_backup_files(directory: &Path, protected_path: &Path) -> std::io::Result<()> {
    let mut entries = fs::read_dir(directory)?
        .map(|entry| {
            let entry = entry?;
            if !entry.file_type()?.is_file() {
                return Ok(None);
            }
            let modified = entry.metadata()?.modified()?;
            Ok(Some((modified, entry.path())))
        })
        .collect::<std::io::Result<Vec<_>>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if entries.len() <= MAX_BACKUPS_PER_FILE {
        return Ok(());
    }
    entries.sort_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| backup_sort_key(&left.1).cmp(&backup_sort_key(&right.1)))
    });
    let mut remaining = entries.len() - MAX_BACKUPS_PER_FILE;
    for (_, path) in entries {
        if remaining == 0 {
            break;
        }
        if path == protected_path {
            continue;
        }
        fs::remove_file(path)?;
        remaining -= 1;
    }
    Ok(())
}

fn backup_sort_key(path: &Path) -> (u8, u64, String, String) {
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if let Some(sequence) = backup_sequence_from_path(path) {
        return (1, sequence, String::new(), file_name.to_owned());
    }
    let stem = file_name.strip_suffix(".yml").unwrap_or(file_name);
    let (process_id, sequence) = stem.rsplit_once('-').unwrap_or((stem, ""));
    (
        0,
        sequence.parse().unwrap_or(u64::MAX),
        process_id.to_owned(),
        file_name.to_owned(),
    )
}

fn cleanup_partial_backup(backup_path: &Path) {
    if let Err(cleanup_error) = fs::remove_file(backup_path)
        && cleanup_error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            operation = "cleanup_partial_models_backup",
            cause = io_error_cause(cleanup_error.kind()),
            "Provider creation backup cleanup failed"
        );
    }
}

fn create_private_backup_directory(
    path: &Path,
    failure: Option<ModelsWriteFailurePoint>,
) -> std::io::Result<()> {
    #[cfg(test)]
    inject_io_failure(
        failure,
        ModelsWriteFailurePoint::BackupDirectoryCreationFailure,
    )?;
    #[cfg(not(test))]
    let _ = failure;
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn allocate_backup_file(
    directory: &Path,
    failure: Option<ModelsWriteFailurePoint>,
) -> std::io::Result<(PathBuf, fs::File)> {
    for _ in 0..BACKUP_ALLOCATION_ATTEMPTS {
        let backup_path = backup_candidate_path(directory, next_backup_sequence(directory)?);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        #[cfg(test)]
        inject_io_failure(failure, ModelsWriteFailurePoint::BackupFileOpenFailure)?;
        match options.open(&backup_path) {
            Ok(backup) => {
                #[cfg(all(test, unix))]
                let _restore_backup_directory_permissions = if failure
                    == Some(ModelsWriteFailurePoint::BackupFilePermissionAndCleanupFailure)
                {
                    Some(restrict_backup_directory_until_drop(directory)?)
                } else {
                    None
                };
                if let Err(error) = set_private_backup_file_permissions(&backup_path, failure) {
                    drop(backup);
                    cleanup_partial_backup(&backup_path);
                    return Err(error);
                }
                return Ok((backup_path, backup));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "could not allocate an unused models.yml backup path",
    ))
}

fn next_backup_sequence(directory: &Path) -> std::io::Result<u64> {
    let mut next = 0;
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        if let Some(sequence) = backup_sequence_from_path(&entry.path()) {
            next = next.max(sequence.saturating_add(1));
        }
    }
    Ok(next)
}

fn set_private_backup_file_permissions(
    path: &Path,
    failure: Option<ModelsWriteFailurePoint>,
) -> std::io::Result<()> {
    #[cfg(all(test, unix))]
    if failure == Some(ModelsWriteFailurePoint::BackupFilePermissionAndCleanupFailure) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "injected backup permission hardening failure",
        ));
    }
    #[cfg(not(test))]
    let _ = failure;
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

#[cfg(all(test, unix))]
struct BackupDirectoryPermissionRestore {
    path: PathBuf,
    permissions: fs::Permissions,
}

#[cfg(all(test, unix))]
impl Drop for BackupDirectoryPermissionRestore {
    fn drop(&mut self) {
        let _ = fs::set_permissions(&self.path, self.permissions.clone());
    }
}

#[cfg(all(test, unix))]
fn restrict_backup_directory_until_drop(
    directory: &Path,
) -> std::io::Result<BackupDirectoryPermissionRestore> {
    let permissions = fs::metadata(directory)?.permissions();
    let mut restricted = permissions.clone();
    restricted.set_mode(permissions.mode() & !0o222);
    fs::set_permissions(directory, restricted)?;
    Ok(BackupDirectoryPermissionRestore {
        path: directory.to_owned(),
        permissions,
    })
}

#[cfg(test)]
fn inject_io_failure(
    failure: Option<ModelsWriteFailurePoint>,
    point: ModelsWriteFailurePoint,
) -> std::io::Result<()> {
    if failure == Some(point) {
        return Err(std::io::Error::other(
            "injected provider creation I/O failure",
        ));
    }
    Ok(())
}

fn backup_candidate_path(directory: &Path, sequence: u64) -> PathBuf {
    directory.join(format!("{sequence:020}.yml"))
}

fn backup_sequence_from_path(path: &Path) -> Option<u64> {
    let file_name = path.file_name()?.to_str()?;
    let stem = file_name.strip_suffix(".yml")?;
    stem.parse().ok()
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
pub(crate) fn target_fingerprint(path: &Path) -> String {
    content_hash(path.to_string_lossy().as_bytes())
}

#[derive(Clone, Copy)]
pub(crate) struct ModelsWriteErrorCodes {
    pub(crate) unavailable: &'static str,
    pub(crate) target_changed: &'static str,
    pub(crate) failed: &'static str,
}

pub(crate) fn remap_models_write_error(error: AppError, codes: ModelsWriteErrorCodes) -> AppError {
    let code = match error.code {
        "provider-create-unavailable" => Some(codes.unavailable),
        "provider-create-target-changed" => Some(codes.target_changed),
        "provider-create-failed"
        | "models-serialize-error"
        | "models-temporary-parse-error"
        | "models-temporary-validation-error"
        | "models-untouched-path-changed" => Some(codes.failed),
        _ => None,
    };
    match code {
        Some(code) => AppError::new(code, error.message, error.action),
        None => error,
    }
}
fn hash_conflict() -> AppError {
    AppError::new(
        "models-hash-conflict",
        "models.yml 在打开表单后已被外部修改。",
        "请重新读取配置；当前表单输入已保留，OMP Switch 不会自动合并。",
    )
}

fn injected_failure(message: impl Into<String>) -> AppError {
    AppError::new(
        "provider-create-failed",
        message,
        "请重试；原 models.yml 没有被修改。",
    )
}

fn write_error(operation: &'static str, error: std::io::Error) -> AppError {
    tracing::warn!(
        operation,
        cause = io_error_cause(error.kind()),
        "Provider creation write failed"
    );
    AppError::new(
        "provider-create-failed",
        "无法安全写入 models.yml。",
        "请检查路径、权限和可用磁盘空间后重试；原文件没有被修改。",
    )
}

fn replacement_outcome_unknown(
    replacement_error: std::io::Error,
    observation_error: Option<std::io::ErrorKind>,
) -> AppError {
    tracing::warn!(
        operation = "reconcile_models_replacement",
        cause = io_error_cause(replacement_error.kind()),
        observation_cause = observation_error.map(io_error_cause),
        status = "unknown",
        "Atomic models replacement outcome could not be confirmed"
    );
    AppError::new(
        "models-replacement-outcome-unknown",
        "无法确认 models.yml 是否已写入。",
        "请重新读取配置并确认 Provider 是否已创建；OMP Switch 不会自动重试。",
    )
}

fn mutate_untouched_value_for_test(tree: &mut Value) {
    let Value::Mapping(root) = tree else {
        return;
    };
    if let Some((key, _)) = root
        .iter()
        .find(|(key, _)| key.as_str() != Some("providers"))
        .map(|(key, value)| (key.clone(), value.clone()))
    {
        root.insert(
            key,
            Value::String("changed-by-failure-injection".to_owned()),
        );
    } else {
        root.insert(
            Value::String("unexpectedRoot".to_owned()),
            Value::String("changed-by-failure-injection".to_owned()),
        );
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn current_backup_retries_when_its_first_candidate_exists() {
        let temporary = tempdir().unwrap();
        let backup_root = temporary.path().join("backups");
        let target_configuration = temporary.path().join("agent");
        let original = b"original models";
        let directory = backup_root
            .join(content_hash(
                target_configuration.to_string_lossy().as_bytes(),
            ))
            .join("models");
        fs::create_dir_all(&directory).unwrap();
        let first_candidate = backup_candidate_path(&directory, 0);
        fs::write(&first_candidate, b"existing backup").unwrap();

        create_current_backup(
            &backup_root,
            &target_configuration,
            "models",
            original,
            None,
        )
        .unwrap();

        assert_eq!(fs::read(first_candidate).unwrap(), b"existing backup");
        assert_eq!(
            fs::read(backup_candidate_path(&directory, 1)).unwrap(),
            original
        );
    }
    #[test]
    fn current_backup_prunes_to_latest_ten_files() {
        let temporary = tempdir().unwrap();
        let backup_root = temporary.path().join("backups");
        let target_configuration = temporary.path().join("agent");
        let directory = backup_root
            .join(content_hash(
                target_configuration.to_string_lossy().as_bytes(),
            ))
            .join("config");

        for _ in 0..11 {
            create_current_backup(
                &backup_root,
                &target_configuration,
                "config",
                b"original config",
                None,
            )
            .unwrap();
        }

        assert_eq!(
            fs::read_dir(&directory).unwrap().count(),
            MAX_BACKUPS_PER_FILE
        );

        let retained = (1..=10)
            .map(|sequence| format!("{sequence:020}.yml"))
            .collect::<std::collections::BTreeSet<_>>();
        let actual = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, retained);
    }

    #[test]
    fn backup_pruning_removes_legacy_names_before_global_names_on_ties() {
        use std::time::{Duration, SystemTime};

        let temporary = tempdir().unwrap();
        let directory = temporary.path().join("config.yml");
        fs::create_dir_all(&directory).unwrap();
        let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        let create_at_timestamp = |path: &Path| {
            let file = fs::File::create(path).unwrap();
            file.set_times(
                fs::FileTimes::new()
                    .set_accessed(timestamp)
                    .set_modified(timestamp),
            )
            .unwrap();
        };
        for sequence in 0..9 {
            create_at_timestamp(&directory.join(format!("1234-{sequence:020}.yml")));
        }
        for sequence in 0..2 {
            create_at_timestamp(&backup_candidate_path(&directory, sequence));
        }
        let protected = backup_candidate_path(&directory, 1);

        prune_backup_files(&directory, &protected).unwrap();

        let actual = fs::read_dir(&directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let mut expected = std::collections::BTreeSet::new();
        for sequence in 1..9 {
            expected.insert(format!("1234-{sequence:020}.yml"));
        }
        expected.insert(format!("{0:020}.yml", 0));
        expected.insert(format!("{0:020}.yml", 1));
        assert_eq!(actual, expected);
    }
    struct PendingTestMutation;

    impl ModelsMutation for PendingTestMutation {
        fn verb(&self) -> &'static str {
            "测试写入"
        }

        fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
            ("pending_test_serialize", "测试序列化失败", "请重试。")
        }

        fn apply(&self, _: &mut Value) -> Result<(), AppError> {
            Ok(())
        }

        fn validate(&self, _: &Value, _: &Value) -> Result<(), AppError> {
            Ok(())
        }
    }

    #[test]
    fn pending_transaction_blocks_single_file_write_after_manifest_exists() {
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("agent");
        fs::create_dir_all(&target).unwrap();
        let target = target.canonicalize().unwrap();
        let original_bytes = b"providers: {}\n".to_vec();
        fs::write(target.join("models.yml"), &original_bytes).unwrap();
        let backup_root = temporary.path().join("backups");
        let transaction_directory = backup_root
            .join(target_fingerprint(&target))
            .join("transactions");
        fs::create_dir_all(&transaction_directory).unwrap();
        fs::write(transaction_directory.join("pending.json"), b"{}").unwrap();
        let loaded = LoadedModels {
            expected_target: target.clone(),
            models_path: target.join("models.yml"),
            backup_partition: "models",
            original_hash: content_hash(&original_bytes),
            original_tree: serde_yaml::from_slice(&original_bytes).unwrap(),
            original_bytes,
        };

        let error =
            write_models_mutation(&backup_root, &loaded, &PendingTestMutation, None).unwrap_err();
        assert_eq!(error.code, "configuration-transaction-pending");
        assert_eq!(
            fs::read(target.join("models.yml")).unwrap(),
            b"providers: {}\n"
        );
    }

    #[test]
    fn config_write_uses_config_backup_partition() {
        let temporary = tempdir().unwrap();
        let target = temporary.path().join("agent");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
        let config_path = target.join("config.yml");
        let config_bytes = b"modelRoles: {}\n";
        fs::write(&config_path, config_bytes).unwrap();
        let discovery =
            crate::target_configuration::discover_target_configuration(&target).unwrap();
        let loaded = load_config_for_write(&discovery, &content_hash(config_bytes)).unwrap();
        let backup_root = temporary.path().join("backups");
        let fingerprint = target_fingerprint(&loaded.expected_target);

        write_models_mutation(&backup_root, &loaded, &PendingTestMutation, None).unwrap();

        let target_root = backup_root.join(fingerprint);
        assert_eq!(fs::read_dir(target_root.join("config")).unwrap().count(), 1);
        assert!(!target_root.join("models").exists());
    }

    #[test]
    fn backup_pruning_orders_same_timestamp_sequences_numerically() {
        use std::time::{Duration, SystemTime};

        let temporary = tempdir().unwrap();
        let directory = temporary.path().join("config.yml");
        fs::create_dir_all(&directory).unwrap();
        let timestamp = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
        for sequence in 0..12 {
            let path = backup_candidate_path(&directory, sequence);
            let file = fs::File::create(&path).unwrap();
            file.set_times(
                fs::FileTimes::new()
                    .set_accessed(timestamp)
                    .set_modified(timestamp),
            )
            .unwrap();
        }
        let protected = backup_candidate_path(&directory, 11);

        prune_backup_files(&directory, &protected).unwrap();

        let actual = fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<std::collections::BTreeSet<_>>();
        let expected = (2..=11)
            .map(|sequence| format!("{sequence:020}.yml"))
            .collect::<std::collections::BTreeSet<_>>();
        assert_eq!(actual, expected);
    }
}
