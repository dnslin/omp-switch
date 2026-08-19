use std::{
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::{
    error::{AppError, io_error_cause},
    models_write,
    redaction::redact_diagnostic,
    target_configuration::{
        ConfigurationFileStatus, TargetConfigurationDiscovery, TargetConfigurationStatus,
    },
};

static TRANSACTION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static RECOVERY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConfigurationTransactionFailurePoint {
    AfterSharedBackup,
    AfterManifest,
    AfterFirstReplacement,
    AfterSecondReplacement,
    DuringCleanup,
}

#[derive(Clone, Debug)]
pub(crate) struct TransactionFile {
    pub(crate) path: PathBuf,
    pub(crate) original_bytes: Vec<u8>,
    pub(crate) original_hash: String,
    pub(crate) original_tree: Value,
}

pub(crate) struct TransactionCandidates {
    pub(crate) models: Value,
    pub(crate) config: Value,
}

struct TemporaryFile {
    writer: AtomicWriteFile,
    tree: Value,
    hash: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionManifest {
    version: u8,
    transaction_id: String,
    logical_target: PathBuf,
    target_fingerprint: String,
    target_configuration: PathBuf,
    entries: Vec<TransactionManifestEntry>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TransactionManifestEntry {
    name: String,
    partition: String,
    target_path: PathBuf,
    backup_path: PathBuf,
    original_hash: String,
    final_hash: String,
}

pub(crate) struct RecoveryResult {
    pub(crate) notice: String,
    pub(crate) manual: bool,
}

pub(crate) fn execute<Prepare, Validate>(
    backup_root: &Path,
    target: &TargetConfigurationDiscovery,
    opened_models_hash: &str,
    opened_config_hash: &str,
    failure: Option<ConfigurationTransactionFailurePoint>,
    operation: &'static str,
    prepare: Prepare,
    validate: Validate,
) -> Result<(), AppError>
where
    Prepare: FnOnce(&TransactionFile, &TransactionFile) -> Result<TransactionCandidates, AppError>,
    Validate: Fn(&TransactionFile, &TransactionFile, &Value, &Value) -> Result<(), AppError>,
{
    validate_transaction_target(target)?;
    let expected_target =
        models_write::resolved_path(&target.resolved_path, "Target configuration")
            .map_err(|_| transaction_unavailable())?;
    let models_path = models_write::resolved_path(&target.models.resolved_path, "models.yml")
        .map_err(|_| transaction_unavailable())?;
    let config_path = models_write::resolved_path(&target.config.resolved_path, "config.yml")
        .map_err(|_| transaction_unavailable())?;
    models_write::ensure_resolved_file_path(&models_path, &expected_target, "models.yml")
        .map_err(|_| transaction_unavailable())?;
    models_write::ensure_resolved_file_path(&config_path, &expected_target, "config.yml")
        .map_err(|_| transaction_unavailable())?;

    let _target_lock = models_write::acquire_configuration_lock(backup_root, &expected_target)
        .map_err(|error| map_lock_error(error, &expected_target))?;
    models_write::ensure_no_pending_configuration_transaction(backup_root, &expected_target)?;
    let mut lock_paths = [models_path.clone(), config_path.clone()];
    lock_paths.sort();
    let mut locks = Vec::with_capacity(lock_paths.len());
    for path in &lock_paths {
        locks.push(
            models_write::acquire_configuration_file_lock(backup_root, &expected_target, path)
                .map_err(|error| map_lock_error(error, path))?,
        );
    }
    ensure_current_target_identity(target, &expected_target, &models_path, &config_path)?;

    let models = load_locked_file(&models_path, opened_models_hash, "models.yml")?;
    let config = load_locked_file(&config_path, opened_config_hash, "config.yml")?;
    let candidates = prepare(&models, &config)?;
    let models_serialized = serialize_candidate(&candidates.models, "models.yml")?;
    let config_serialized = serialize_candidate(&candidates.config, "config.yml")?;
    let transaction_id = next_transaction_id();

    let models_backup = models_write::create_transaction_backup(
        backup_root,
        &expected_target,
        &models.original_bytes,
        "models",
        &transaction_id,
    )
    .map_err(map_backup_error)?;
    let config_backup = match models_write::create_transaction_backup(
        backup_root,
        &expected_target,
        &config.original_bytes,
        "config",
        &transaction_id,
    ) {
        Ok(path) => path,
        Err(error) => {
            tracing::warn!(
                operation = "cleanup_partial_configuration_transaction_backup",
                cause = error.code,
                "Configuration transaction shared backup was incomplete"
            );
            if let Err(cleanup_error) = fs::remove_file(&models_backup) {
                tracing::warn!(
                    operation = "cleanup_partial_configuration_transaction_backup_file",
                    cause = io_error_cause(cleanup_error.kind()),
                    "Configuration transaction first backup cleanup failed"
                );
            }
            return Err(map_backup_error(error));
        }
    };
    if failure == Some(ConfigurationTransactionFailurePoint::AfterSharedBackup) {
        return Err(injected_failure(operation, "共享备份后"));
    }

    let temporary_models = write_temporary(&models_path, models_serialized, "models.yml")?;
    let temporary_config = match write_temporary(&config_path, config_serialized, "config.yml") {
        Ok(file) => file,
        Err(error) => {
            discard_temporary(temporary_models);
            return Err(error);
        }
    };
    validate(
        &models,
        &config,
        &temporary_models.tree,
        &temporary_config.tree,
    )?;
    ensure_hashes_unchanged(
        &models_path,
        &config_path,
        &models.original_hash,
        &config.original_hash,
    )?;
    models_write::ensure_resolved_file_path(&models_path, &expected_target, "models.yml")
        .map_err(|_| transaction_unavailable())?;
    models_write::ensure_resolved_file_path(&config_path, &expected_target, "config.yml")
        .map_err(|_| transaction_unavailable())?;
    ensure_current_target_identity(target, &expected_target, &models_path, &config_path)?;

    let manifest = TransactionManifest {
        version: 1,
        transaction_id: transaction_id.clone(),
        logical_target: PathBuf::from(&target.path),
        target_fingerprint: models_write::target_fingerprint(&expected_target),
        target_configuration: expected_target.clone(),
        entries: vec![
            TransactionManifestEntry {
                name: "models.yml".to_owned(),
                partition: "models".to_owned(),
                target_path: models_path.clone(),
                backup_path: models_backup,
                original_hash: models.original_hash.clone(),
                final_hash: temporary_models.hash.clone(),
            },
            TransactionManifestEntry {
                name: "config.yml".to_owned(),
                partition: "config".to_owned(),
                target_path: config_path.clone(),
                backup_path: config_backup,
                original_hash: config.original_hash.clone(),
                final_hash: temporary_config.hash.clone(),
            },
        ],
    };
    let manifest_path = persist_manifest(backup_root, &expected_target, &manifest)?;
    if failure == Some(ConfigurationTransactionFailurePoint::AfterManifest) {
        discard_temporary(temporary_models);
        discard_temporary(temporary_config);
        return Err(injected_failure(operation, "事务清单后"));
    }

    let models_commit = temporary_models.writer.commit();
    if let Err(error) = models_commit {
        return Err(commit_error(operation, "models.yml", error));
    }
    if failure == Some(ConfigurationTransactionFailurePoint::AfterFirstReplacement) {
        return Err(injected_failure(operation, "第一文件替换后"));
    }
    let config_commit = temporary_config.writer.commit();
    if let Err(error) = config_commit {
        return Err(commit_error(operation, "config.yml", error));
    }
    if failure == Some(ConfigurationTransactionFailurePoint::AfterSecondReplacement) {
        return Err(injected_failure(operation, "第二文件替换后"));
    }
    if failure == Some(ConfigurationTransactionFailurePoint::DuringCleanup) {
        return Err(injected_failure(operation, "事务清理时"));
    }
    cleanup_manifest_after_commit(&manifest_path);
    drop(locks);
    Ok(())
}

fn validate_transaction_target(target: &TargetConfigurationDiscovery) -> Result<(), AppError> {
    if target.status != TargetConfigurationStatus::Writable || !target.writable {
        return Err(transaction_unavailable());
    }
    if !matches!(
        target.models.status,
        ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
    ) || !matches!(
        target.config.status,
        ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
    ) {
        return Err(transaction_unavailable());
    }
    Ok(())
}
fn ensure_current_target_identity(
    target: &TargetConfigurationDiscovery,
    expected_target: &Path,
    expected_models: &Path,
    expected_config: &Path,
) -> Result<(), AppError> {
    for (label, logical_path, expected_path) in [
        (
            "Target configuration",
            PathBuf::from(&target.path),
            expected_target.to_path_buf(),
        ),
        (
            "models.yml",
            PathBuf::from(&target.models.canonical_path),
            expected_models.to_path_buf(),
        ),
        (
            "config.yml",
            PathBuf::from(&target.config.canonical_path),
            expected_config.to_path_buf(),
        ),
    ] {
        let current_path = logical_path
            .canonicalize()
            .map_err(|_| target_identity_conflict(label))?;
        if current_path != expected_path {
            return Err(target_identity_conflict(label));
        }
    }
    Ok(())
}

fn target_identity_conflict(label: &str) -> AppError {
    AppError::new(
        "configuration-transaction-target-changed",
        format!("{label} 的真实文件系统目标已变化。"),
        "请重新读取配置并重新打开删除确认；本次操作没有修改两个目标文件。",
    )
}

fn load_locked_file(
    path: &Path,
    expected_hash: &str,
    label: &'static str,
) -> Result<TransactionFile, AppError> {
    let bytes = fs::read(path)
        .map_err(|error| transaction_io_error("读取 Configuration transaction 目标", error))?;
    let actual_hash = models_write::content_hash(&bytes);
    if actual_hash != expected_hash {
        return Err(hash_conflict(label));
    }
    let tree = serde_yaml::from_slice(&bytes).map_err(|error| {
        AppError::new(
            "configuration-transaction-parse-error",
            format!(
                "{label} 无法重新解析：{}",
                redact_diagnostic(&error.to_string())
            ),
            "请在外部修复 YAML 后重新读取；本次操作没有写入目标文件。",
        )
    })?;
    Ok(TransactionFile {
        path: path.to_owned(),
        original_bytes: bytes,
        original_hash: actual_hash,
        original_tree: tree,
    })
}

fn serialize_candidate(tree: &Value, label: &'static str) -> Result<Vec<u8>, AppError> {
    serde_yaml::to_string(tree)
        .map(|value| value.into_bytes())
        .map_err(|error| {
            AppError::new(
                "configuration-transaction-serialize-error",
                format!(
                    "无法序列化 {label} 的 Configuration transaction 结果：{}",
                    redact_diagnostic(&error.to_string())
                ),
                "请重试；两个原始配置文件都没有被修改。",
            )
        })
}

fn write_temporary(
    path: &Path,
    bytes: Vec<u8>,
    label: &'static str,
) -> Result<TemporaryFile, AppError> {
    let mut writer = AtomicWriteFile::options()
        .read(true)
        .open(path)
        .map_err(|error| transaction_io_error("创建 Configuration transaction 临时文件", error))?;
    writer
        .write_all(&bytes)
        .map_err(|error| transaction_io_error("写入 Configuration transaction 临时文件", error))?;
    writer
        .sync_all()
        .map_err(|error| transaction_io_error("同步 Configuration transaction 临时文件", error))?;
    writer
        .seek(SeekFrom::Start(0))
        .map_err(|error| transaction_io_error("读取 Configuration transaction 临时文件", error))?;
    let mut actual_bytes = Vec::new();
    writer
        .read_to_end(&mut actual_bytes)
        .map_err(|error| transaction_io_error("读取 Configuration transaction 临时文件", error))?;
    let tree = serde_yaml::from_slice::<Value>(&actual_bytes).map_err(|error| {
        AppError::new(
            "configuration-transaction-temporary-parse-error",
            format!(
                "临时 {label} 无法重新解析：{}",
                redact_diagnostic(&error.to_string())
            ),
            "请重试；两个原始配置文件都没有被修改。",
        )
    })?;
    Ok(TemporaryFile {
        writer,
        hash: models_write::content_hash(&actual_bytes),
        tree,
    })
}

fn discard_temporary(temporary: TemporaryFile) {
    if let Err(error) = temporary.writer.discard() {
        tracing::warn!(
            operation = "cleanup_configuration_transaction_temporary",
            cause = io_error_cause(error.kind()),
            "Configuration transaction temporary cleanup failed"
        );
    }
}

fn ensure_hashes_unchanged(
    models_path: &Path,
    config_path: &Path,
    models_hash: &str,
    config_hash: &str,
) -> Result<(), AppError> {
    let current_models = fs::read(models_path)
        .map_err(|error| transaction_io_error("重新读取 models.yml", error))?;
    if models_write::content_hash(&current_models) != models_hash {
        return Err(hash_conflict("models.yml"));
    }
    let current_config = fs::read(config_path)
        .map_err(|error| transaction_io_error("重新读取 config.yml", error))?;
    if models_write::content_hash(&current_config) != config_hash {
        return Err(hash_conflict("config.yml"));
    }
    Ok(())
}

fn transaction_manifest_file_name(logical_target: &Path, transaction_id: &str) -> String {
    format!(
        "{}-{transaction_id}.json",
        models_write::target_fingerprint(logical_target)
    )
}

fn persist_manifest(
    backup_root: &Path,
    target: &Path,
    manifest: &TransactionManifest,
) -> Result<PathBuf, AppError> {
    let directory = backup_root
        .join(&manifest.target_fingerprint)
        .join("transactions");
    fs::create_dir_all(&directory)
        .map_err(|error| transaction_io_error("创建 Configuration transaction 清单目录", error))?;
    let path = directory.join(transaction_manifest_file_name(
        &manifest.logical_target,
        &manifest.transaction_id,
    ));
    let bytes = serde_json::to_vec(manifest).map_err(|error| {
        AppError::new(
            "configuration-transaction-manifest-error",
            format!(
                "无法生成 Configuration transaction 清单：{}",
                redact_diagnostic(&error.to_string())
            ),
            "请重试；两个原始配置文件都没有被修改。",
        )
    })?;
    let mut writer = AtomicWriteFile::options()
        .read(true)
        .open(&path)
        .map_err(|error| transaction_io_error("创建 Configuration transaction 清单", error))?;
    writer
        .write_all(&bytes)
        .and_then(|_| writer.sync_all())
        .and_then(|_| writer.commit())
        .map_err(|error| transaction_io_error("持久化 Configuration transaction 清单", error))?;
    let _ = target;
    Ok(path)
}

fn cleanup_manifest_after_commit(path: &Path) {
    if let Err(error) = fs::remove_file(path)
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!(
            operation = "cleanup_configuration_transaction_manifest",
            cause = io_error_cause(error.kind()),
            "Configuration transaction manifest cleanup failed"
        );
    }
}

fn next_transaction_id() -> String {
    let sequence = TRANSACTION_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!(
        "{}-{:016x}-{:016x}",
        std::process::id(),
        timestamp,
        sequence
    )
}

fn hash_conflict(label: &'static str) -> AppError {
    let code = if label == "models.yml" {
        "models-hash-conflict"
    } else {
        "config-hash-conflict"
    };
    AppError::new(
        code,
        format!("{label} 在 Configuration transaction 提交前已被外部修改。"),
        "请重新读取配置；当前删除确认仍保留，OMP Switch 不会自动合并。",
    )
}

fn transaction_unavailable() -> AppError {
    AppError::new(
        "configuration-transaction-unavailable",
        "当前 Target configuration 不允许安全执行跨文件删除。",
        "请重新检测 OMP，并处理只读、解析错误、链接或权限问题后重试。",
    )
}

fn map_lock_error(error: AppError, path: &Path) -> AppError {
    if error.code == "models-write-in-progress" {
        return AppError::new(
            "configuration-transaction-in-progress",
            format!("Configuration transaction 正在锁定 {}。", path.display()),
            "请等待当前写入完成后重新读取配置，再重试。",
        );
    }
    transaction_unavailable()
}

fn map_backup_error(_error: AppError) -> AppError {
    AppError::new(
        "configuration-transaction-backup-failed",
        "无法为两个配置文件创建同一 Configuration transaction 备份。",
        "请检查路径、权限和可用磁盘空间；本次操作没有替换任一目标文件。",
    )
}

fn injected_failure(operation: &str, point: &str) -> AppError {
    AppError::new(
        "configuration-transaction-interrupted",
        format!("{operation}在{point}发生中断，事务清单已保留。"),
        "请重新检测 OMP；启动流程将根据最终 Hash 完成提交清理或整体恢复。",
    )
}

fn commit_error(operation: &str, label: &str, error: std::io::Error) -> AppError {
    tracing::warn!(
        operation = "configuration_transaction_replacement",
        cause = io_error_cause(error.kind()),
        target = label,
        "Configuration transaction replacement failed; manifest retained for recovery"
    );
    AppError::new(
        "configuration-transaction-replacement-failed",
        format!("{operation}替换 {label} 时发生错误。"),
        "请重新检测 OMP；启动流程将根据事务清单完成提交清理或整体恢复。",
    )
}

fn transaction_io_error(operation: &str, error: std::io::Error) -> AppError {
    tracing::warn!(
        operation,
        cause = io_error_cause(error.kind()),
        "Configuration transaction I/O failed"
    );
    AppError::new(
        "configuration-transaction-failed",
        "Configuration transaction 无法安全完成。",
        "请检查路径、权限和可用磁盘空间；原始目标文件没有被安全替换。",
    )
}

enum ManifestCandidate {
    Valid {
        path: PathBuf,
        directory: PathBuf,
        manifest: TransactionManifest,
        bytes: Vec<u8>,
    },
    Invalid {
        path: PathBuf,
        detail: String,
    },
}

pub(crate) fn recover_for_target(
    backup_root: &Path,
    logical_target: &Path,
) -> std::io::Result<Option<RecoveryResult>> {
    let mut candidates = find_manifests_for_logical_target(backup_root, logical_target)?;
    if candidates.is_empty() {
        return Ok(None);
    }
    candidates.sort_by_key(|candidate| match candidate {
        ManifestCandidate::Valid { path, .. } | ManifestCandidate::Invalid { path, .. } => {
            path.clone()
        }
    });
    let candidate = candidates.remove(0);
    let (manifest_path, directory, manifest, manifest_bytes) = match candidate {
        ManifestCandidate::Valid {
            path,
            directory,
            manifest,
            bytes,
        } => (path, directory, manifest, bytes),
        ManifestCandidate::Invalid { path, detail } => {
            let scene = preserve_current_scene(logical_target, &path)?;
            return Ok(Some(manual_recovery(
                &scene,
                format!("{detail}；原清单：{}", path.display()),
            )));
        }
    };
    let target = manifest.target_configuration.clone();
    if !candidates.is_empty() {
        let invalid_paths = candidates
            .iter()
            .filter_map(|candidate| match candidate {
                ManifestCandidate::Invalid { path, .. } => Some(path.display().to_string()),
                ManifestCandidate::Valid { .. } => None,
            })
            .collect::<Vec<_>>();
        let detail = if invalid_paths.is_empty() {
            String::new()
        } else {
            format!(" 无法读取或解析的清单：{}。", invalid_paths.join("、"))
        };
        let scene = preserve_current_scene(&target, &manifest_path)?;
        return Ok(Some(manual_recovery(
            &scene,
            format!(
                "发现 {} 份未完成 Configuration transaction 清单，无法确定恢复顺序。{detail}",
                candidates.len() + 1
            ),
        )));
    }
    if let Err(error) = validate_manifest(
        &manifest,
        &target,
        &directory,
        logical_target,
        &manifest_path,
    ) {
        let scene = preserve_current_scene(&target, &manifest_path)?;
        return Ok(Some(manual_recovery(
            &scene,
            format!(
                "事务清单验证失败：{}；原清单：{}",
                redact_diagnostic(&error),
                manifest_path.display()
            ),
        )));
    }

    let _target_lock = models_write::acquire_configuration_lock(backup_root, &target)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    let mut lock_paths = manifest
        .entries
        .iter()
        .map(|entry| entry.target_path.clone())
        .collect::<Vec<_>>();
    lock_paths.sort();
    lock_paths.dedup();
    let mut locks = Vec::with_capacity(lock_paths.len());
    for path in &lock_paths {
        locks.push(
            models_write::acquire_configuration_file_lock(backup_root, &target, path)
                .map_err(|error| std::io::Error::other(error.to_string()))?,
        );
    }
    if let Err(error) = validate_manifest(
        &manifest,
        &target,
        &directory,
        logical_target,
        &manifest_path,
    ) {
        let scene = preserve_current_scene(&target, &manifest_path)?;
        return Ok(Some(manual_recovery(
            &scene,
            format!(
                "事务清单锁定后验证失败：{}；原清单：{}",
                redact_diagnostic(&error),
                manifest_path.display()
            ),
        )));
    }
    let locked_bytes = match fs::read(&manifest_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            let scene = preserve_current_scene(&target, &manifest_path)?;
            return Ok(Some(manual_recovery(
                &scene,
                format!(
                    "事务清单锁定后无法重新读取：{}；原清单：{}",
                    redact_diagnostic(&error.to_string()),
                    manifest_path.display()
                ),
            )));
        }
    };
    let locked_manifest = match serde_json::from_slice::<TransactionManifest>(&locked_bytes) {
        Ok(manifest) => manifest,
        Err(error) => {
            let scene = preserve_current_scene(&target, &manifest_path)?;
            return Ok(Some(manual_recovery(
                &scene,
                format!(
                    "事务清单锁定后无法解析：{}；原清单：{}",
                    redact_diagnostic(&error.to_string()),
                    manifest_path.display()
                ),
            )));
        }
    };
    if locked_bytes != manifest_bytes || locked_manifest != manifest {
        let scene = preserve_current_scene(&target, &manifest_path)?;
        return Ok(Some(manual_recovery(
            &scene,
            format!(
                "事务清单锁定后内容发生变化；原清单：{}",
                manifest_path.display()
            ),
        )));
    }
    let manifest = locked_manifest;
    let all_final = manifest.entries.iter().all(|entry| {
        fs::read(&entry.target_path)
            .map(|bytes| models_write::content_hash(&bytes) == entry.final_hash)
            .unwrap_or(false)
    });
    if all_final {
        if let Err(error) = fs::remove_file(&manifest_path) {
            return Ok(Some(manual_recovery(
                &manifest_path,
                format!(
                    "完整提交已确认，但事务清单清理失败：{}",
                    redact_diagnostic(&error.to_string())
                ),
            )));
        }
        return Ok(Some(RecoveryResult {
            notice: format!(
                "上次 Configuration transaction 已完整提交；已按最终 Hash 清理事务清单。共享备份保留在 {}。",
                directory.display()
            ),
            manual: false,
        }));
    }
    let scene = preserve_current_scene(&target, &manifest_path)?;
    let mut backups = Vec::with_capacity(manifest.entries.len());
    let mut backup_errors = Vec::new();
    for entry in &manifest.entries {
        match fs::read(&entry.backup_path) {
            Ok(bytes) if models_write::content_hash(&bytes) == entry.original_hash => {
                backups.push((entry, bytes));
            }
            Ok(_) => backup_errors.push(format!("{} 的备份 Hash 不匹配", entry.name)),
            Err(error) => backup_errors.push(format!("{} 备份不可读：{}", entry.name, error)),
        }
    }
    if !backup_errors.is_empty() {
        return Ok(Some(manual_recovery(
            &scene,
            format!("整体恢复前备份预检失败：{}", backup_errors.join("；")),
        )));
    }
    let mut restore_errors = Vec::new();
    for (entry, backup) in &backups {
        if let Err(error) = restore_file(&entry.target_path, backup) {
            restore_errors.push(format!("{} 恢复失败：{}", entry.name, error));
        }
    }
    for entry in &manifest.entries {
        match fs::read(&entry.target_path) {
            Ok(bytes) if models_write::content_hash(&bytes) == entry.original_hash => {}
            Ok(_) => restore_errors.push(format!("{} 恢复后的 Hash 不匹配", entry.name)),
            Err(error) => restore_errors.push(format!("{} 恢复后不可读：{}", entry.name, error)),
        }
    }
    if !restore_errors.is_empty() {
        return Ok(Some(manual_recovery(
            &scene,
            format!("整体恢复未完成：{}", restore_errors.join("；")),
        )));
    }
    if let Err(error) = fs::remove_file(&manifest_path) {
        return Ok(Some(manual_recovery(
            &scene,
            format!(
                "整体恢复成功，但事务清单清理失败：{}",
                redact_diagnostic(&error.to_string())
            ),
        )));
    }
    drop(locks);
    Ok(Some(RecoveryResult {
        notice: format!(
            "上次 Configuration transaction 未完整提交；已将现场保存到 {}，并从同一事务备份整体恢复 models.yml 与 config.yml。",
            scene.display()
        ),
        manual: false,
    }))
}
fn find_manifests_for_logical_target(
    backup_root: &Path,
    logical_target: &Path,
) -> std::io::Result<Vec<ManifestCandidate>> {
    let target_roots = match fs::read_dir(backup_root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let prefix = format!("{}-", models_write::target_fingerprint(logical_target));
    let mut matches = Vec::new();
    for entry in target_roots {
        let target_root = entry?.path();
        if !target_root.is_dir() {
            continue;
        }
        let transaction_directory = target_root.join("transactions");
        let manifests = match fs::read_dir(&transaction_directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for manifest_entry in manifests {
            let manifest_path = manifest_entry?.path();
            let Some(file_name) = manifest_path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            if !file_name.starts_with(&prefix)
                || manifest_path.extension().and_then(|value| value.to_str()) != Some("json")
            {
                continue;
            }
            let bytes = match fs::read(&manifest_path) {
                Ok(bytes) => bytes,
                Err(error) => {
                    matches.push(ManifestCandidate::Invalid {
                        path: manifest_path,
                        detail: format!("事务清单读取失败：{}", error.kind()),
                    });
                    continue;
                }
            };
            match serde_json::from_slice::<TransactionManifest>(&bytes) {
                Ok(manifest) => matches.push(ManifestCandidate::Valid {
                    path: manifest_path,
                    directory: transaction_directory.clone(),
                    manifest,
                    bytes,
                }),
                Err(error) => matches.push(ManifestCandidate::Invalid {
                    path: manifest_path,
                    detail: format!(
                        "事务清单无法解析：{}",
                        redact_diagnostic(&error.to_string())
                    ),
                }),
            }
        }
    }
    Ok(matches)
}

pub(crate) fn remove_model_role_ids(tree: &mut Value, role_ids: &[String]) -> Result<(), AppError> {
    let roles = tree
        .as_mapping_mut()
        .and_then(|root| root.get_mut(Value::String("modelRoles".to_owned())))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(|| {
            AppError::new(
                "configuration-transaction-roles-invalid",
                "config.yml 的 modelRoles 结构无法安全修改。",
                "请在外部修复后重新读取；两个原始配置文件都没有被修改。",
            )
        })?;
    for role_id in role_ids {
        roles.remove(Value::String(role_id.clone()));
    }
    Ok(())
}

pub(crate) fn validate_model_role_ids_removed(
    original: &Value,
    candidate: &Value,
    role_ids: &[String],
) -> Result<(), AppError> {
    let original_root = original
        .as_mapping()
        .ok_or_else(|| transaction_roles_untouched_error())?;
    let candidate_root = candidate
        .as_mapping()
        .ok_or_else(|| transaction_roles_untouched_error())?;
    if original_root.len() != candidate_root.len() {
        return Err(transaction_roles_untouched_error());
    }
    for (key, value) in original_root {
        if key.as_str() != Some("modelRoles") && candidate_root.get(key) != Some(value) {
            return Err(transaction_roles_untouched_error());
        }
    }
    let original_roles = original_root
        .get(Value::String("modelRoles".to_owned()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| transaction_roles_untouched_error())?;
    let candidate_roles = candidate_root
        .get(Value::String("modelRoles".to_owned()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| transaction_roles_untouched_error())?;
    let mut expected_roles = original_roles.clone();
    for role_id in role_ids {
        expected_roles.remove(Value::String(role_id.clone()));
    }
    if candidate_roles != &expected_roles {
        return Err(transaction_roles_untouched_error());
    }
    Ok(())
}

fn transaction_roles_untouched_error() -> AppError {
    AppError::new(
        "configuration-transaction-untouched-path-changed",
        "Configuration transaction 改变了未触及的 config.yml 路径。",
        "请重试；两个原始配置文件都没有被安全替换。",
    )
}

fn validate_manifest(
    manifest: &TransactionManifest,
    target: &Path,
    transaction_directory: &Path,
    logical_target: &Path,
    manifest_path: &Path,
) -> Result<(), String> {
    if manifest.version != 1 || manifest.transaction_id.is_empty() {
        return Err("事务版本或 ID 无效".to_owned());
    }
    if manifest.logical_target != logical_target
        || manifest_path.file_name().and_then(|value| value.to_str())
            != Some(
                transaction_manifest_file_name(logical_target, &manifest.transaction_id).as_str(),
            )
    {
        return Err("事务逻辑 Target 或清单文件名不匹配".to_owned());
    }
    if manifest.target_configuration != target
        || manifest.target_fingerprint != models_write::target_fingerprint(target)
        || manifest.entries.len() != 2
    {
        return Err("事务目标不匹配".to_owned());
    }
    let backup_root = transaction_directory
        .parent()
        .ok_or_else(|| "事务备份根目录无效".to_owned())?;
    for (expected_name, expected_partition) in [("models.yml", "models"), ("config.yml", "config")]
    {
        let Some(entry) = manifest
            .entries
            .iter()
            .find(|entry| entry.name == expected_name)
        else {
            return Err(format!("事务缺少 {expected_name}"));
        };
        let expected_target_path = target
            .join(expected_name)
            .canonicalize()
            .map_err(|error| format!("{expected_name} 真实路径无法解析：{}", error.kind()))?;
        if !fs::metadata(&expected_target_path)
            .map_err(|error| format!("{expected_name} 真实路径无法读取：{}", error.kind()))?
            .is_file()
        {
            return Err(format!("{expected_name} 真实路径不是普通文件"));
        }
        if entry.partition != expected_partition
            || entry.target_path != expected_target_path
            || entry.backup_path
                != backup_root
                    .join(expected_partition)
                    .join(format!("tx-{}.yml", manifest.transaction_id))
            || !is_hash(&entry.original_hash)
            || !is_hash(&entry.final_hash)
        {
            return Err(format!("{expected_name} 的路径、分区或 Hash 无效"));
        }
    }
    Ok(())
}

fn is_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn preserve_current_scene(target: &Path, manifest_path: &Path) -> std::io::Result<PathBuf> {
    let sequence = RECOVERY_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let root = manifest_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("recovery")
        .join(format!("{}-{sequence:016x}", std::process::id()));
    fs::create_dir_all(&root)?;
    #[cfg(unix)]
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))?;
    for name in ["models.yml", "config.yml"] {
        let source = target.join(name);
        let destination = root.join(name);
        if source.exists() {
            fs::copy(source, destination)?;
        }
    }
    Ok(root)
}

fn restore_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut writer = AtomicWriteFile::options().read(true).open(path)?;
    writer.write_all(bytes)?;
    writer.sync_all()?;
    writer.commit()
}

fn manual_recovery(path: &Path, detail: String) -> RecoveryResult {
    RecoveryResult {
        notice: format!(
            "Configuration transaction 需要人工处理；当前 Target configuration 已锁定为只读，未确认整体恢复。安全路径：{}。详情：{}。请处理后重新检测。",
            path.display(),
            redact_diagnostic(&detail)
        ),
        manual: true,
    }
}

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
