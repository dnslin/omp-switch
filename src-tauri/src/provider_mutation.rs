use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

use fs4::{FileExt, TryLockError};

use atomic_write_file::AtomicWriteFile;
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    bundled_catalog::BundledCatalog,
    error::{AppError, io_error_cause},
    redaction::redact_diagnostic,
    target_configuration::{
        ConfigurationFileStatus, TargetConfigurationDiscovery, TargetConfigurationStatus,
    },
};

static BACKUP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const BACKUP_ALLOCATION_ATTEMPTS: usize = 128;

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
pub(crate) enum SupportedApi {
    #[serde(rename = "openai-completions")]
    OpenAiCompletions,
    #[serde(rename = "openai-responses")]
    OpenAiResponses,
    #[serde(rename = "anthropic-messages")]
    AnthropicMessages,
    #[serde(rename = "google-generative-ai")]
    GoogleGenerativeAi,
}

impl SupportedApi {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompletions => "openai-completions",
            Self::OpenAiResponses => "openai-responses",
            Self::AnthropicMessages => "anthropic-messages",
            Self::GoogleGenerativeAi => "google-generative-ai",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProviderAuthMode {
    ApiKey,
    None,
}

#[derive(Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum SupportedInput {
    Text,
    Image,
}

impl SupportedInput {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateProviderFields {
    pub(crate) id: String,
    pub(crate) base_url: String,
    pub(crate) default_api: Option<SupportedApi>,
    pub(crate) auth_mode: ProviderAuthMode,
    pub(crate) api_key: Option<String>,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateModelFields {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) api: Option<SupportedApi>,
    pub(crate) reasoning: bool,
    pub(crate) input: Vec<SupportedInput>,
    pub(crate) context_window: u64,
    pub(crate) max_tokens: u64,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateCustomProviderInput {
    pub(crate) opened_models_hash: String,
    pub(crate) provider: CreateProviderFields,
    pub(crate) first_model: CreateModelFields,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateCustomProviderResult {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum DirectApiKeyIntent {
    Keep,
    Replace { value: String },
    Delete,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EditCustomProviderInput {
    pub(crate) opened_models_hash: String,
    pub(crate) provider_id: String,
    pub(crate) base_url: String,
    pub(crate) default_api: Option<SupportedApi>,
    pub(crate) auth_mode: ProviderAuthMode,
    pub(crate) api_key: DirectApiKeyIntent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EditCustomProviderResult {
    pub(crate) provider_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderMutationFailurePoint {
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

struct ValidatedCreate {
    provider_id: String,
    model_id: String,
    provider_value: Value,
}

struct ValidatedEdit {
    provider_id: String,
    base_url: String,
    default_api: Option<SupportedApi>,
    api_key: ValidatedApiKeyIntent,
}

enum ValidatedApiKeyIntent {
    Keep,
    Replace(String),
    Delete,
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

impl ModelsMutation for ValidatedCreate {
    fn verb(&self) -> &'static str {
        "创建"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "serialize_created_provider",
            "无法序列化新的 Provider 配置",
            "请检查表单后重试；原 models.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        insert_provider(tree, &self.provider_id, self.provider_value.clone())
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        validate_created_provider(candidate, self)?;
        ensure_untouched_paths_equal(original, candidate, &self.provider_id)
    }
}

impl ModelsMutation for ValidatedEdit {
    fn verb(&self) -> &'static str {
        "编辑"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "serialize_edited_provider",
            "无法序列化 Provider 编辑结果",
            "请检查表单后重试；原 models.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        update_provider(tree, self)
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        validate_edited_provider(candidate, original, self)?;
        ensure_edited_untouched_paths_equal(
            original,
            candidate,
            &self.provider_id,
            &["baseUrl", "api", "apiKey"],
        )
    }
}

pub(crate) struct LoadedModels {
    pub(crate) expected_target: PathBuf,
    pub(crate) models_path: PathBuf,
    pub(crate) original_bytes: Vec<u8>,
    pub(crate) original_hash: String,
    pub(crate) original_tree: Value,
}

pub(crate) fn create_custom_provider(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &CreateCustomProviderInput,
    failure: Option<ProviderMutationFailurePoint>,
) -> Result<CreateCustomProviderResult, AppError> {
    let loaded = load_models_for_write(target, &input.opened_models_hash)?;
    let validated = validate_input(input, &loaded.original_tree, catalog)?;
    let result = CreateCustomProviderResult {
        provider_id: validated.provider_id.clone(),
        model_id: validated.model_id.clone(),
    };
    write_models_mutation(backup_root, &loaded, &validated, failure)?;
    Ok(result)
}

pub(crate) fn edit_custom_provider(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &EditCustomProviderInput,
    failure: Option<ProviderMutationFailurePoint>,
) -> Result<EditCustomProviderResult, AppError> {
    let loaded = load_models_for_write(target, &input.opened_models_hash)
        .map_err(remap_edit_operation_error)?;
    let validated = validate_edit_input(input, &loaded.original_tree, catalog)?;
    let result = EditCustomProviderResult {
        provider_id: validated.provider_id.clone(),
    };
    write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(remap_edit_operation_error)?;
    Ok(result)
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
        original_bytes,
        original_hash,
        original_tree,
    })
}

pub(crate) fn write_models_mutation<M: ModelsMutation>(
    backup_root: &Path,
    loaded: &LoadedModels,
    mutation: &M,
    failure: Option<ProviderMutationFailurePoint>,
) -> Result<(), AppError> {
    let verb = mutation.verb();
    if failure == Some(ProviderMutationFailurePoint::BeforeBackup) {
        return Err(injected_failure(format!("{verb}当前备份前发生故障。")));
    }
    let _write_lock = acquire_models_write_lock(backup_root, &loaded.expected_target)?;
    create_current_backup(
        backup_root,
        &loaded.expected_target,
        &loaded.original_bytes,
        failure,
    )?;
    if failure == Some(ProviderMutationFailurePoint::AfterBackup) {
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

    if failure == Some(ProviderMutationFailurePoint::BeforeTemporaryWrite) {
        return Err(injected_failure(format!("{verb}临时文件前发生故障。")));
    }
    #[cfg(test)]
    inject_io_failure(
        failure,
        ProviderMutationFailurePoint::TemporaryFileOpenFailure,
    )
    .map_err(|error| write_error("open_models_temporary", error))?;
    let mut temporary = AtomicWriteFile::options()
        .read(true)
        .open(&loaded.models_path)
        .map_err(|error| write_error("open_models_temporary", error))?;
    #[cfg(test)]
    inject_io_failure(
        failure,
        ProviderMutationFailurePoint::TemporaryFileWriteFailure,
    )
    .map_err(|error| write_error("write_models_temporary", error))?;
    temporary
        .write_all(serialized.as_bytes())
        .map_err(|error| write_error("write_models_temporary", error))?;
    #[cfg(test)]
    inject_io_failure(
        failure,
        ProviderMutationFailurePoint::TemporaryFileSyncFailure,
    )
    .map_err(|error| write_error("write_models_temporary", error))?;
    temporary
        .sync_all()
        .map_err(|error| write_error("write_models_temporary", error))?;
    if failure == Some(ProviderMutationFailurePoint::CorruptTemporaryFile) {
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
    if failure == Some(ProviderMutationFailurePoint::MutateUntouchedValue) {
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
    if failure == Some(ProviderMutationFailurePoint::MutateConfigBeforeReplacement) {
        mutation.mutate_external_state_for_test();
    }
    mutation.validate_before_commit(loaded)?;
    if failure == Some(ProviderMutationFailurePoint::BeforeReplacement) {
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
        if failure == Some(ProviderMutationFailurePoint::AfterAtomicReplacement) {
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
    failure: Option<ProviderMutationFailurePoint>,
) -> Result<Option<(PathBuf, fs::Permissions)>, AppError> {
    if failure != Some(ProviderMutationFailurePoint::CommitFailure) {
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
fn acquire_models_write_lock(
    backup_root: &Path,
    resolved_target: &Path,
) -> Result<fs::File, AppError> {
    let lock_directory = backup_root.join(".locks");
    create_private_backup_directory(&lock_directory, None)
        .map_err(|error| write_error("create_models_lock_directory", error))?;
    let target_fingerprint = content_hash(resolved_target.to_string_lossy().as_bytes());
    let lock_path = lock_directory.join(format!("{target_fingerprint}.lock"));
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true);
    #[cfg(unix)]
    options.mode(0o600);
    let lock = options
        .open(&lock_path)
        .map_err(|error| write_error("open_models_write_lock", error))?;
    set_private_backup_file_permissions(&lock_path, None)
        .map_err(|error| write_error("secure_models_write_lock", error))?;
    match FileExt::try_lock(&lock) {
        Ok(()) => Ok(lock),
        Err(TryLockError::WouldBlock) => Err(models_write_in_progress()),
        Err(TryLockError::Error(error)) => Err(write_error("lock_models_write", error)),
    }
}

fn models_write_in_progress() -> AppError {
    AppError::new(
        "models-write-in-progress",
        "另一项 OMP Switch Provider 创建正在写入 models.yml。",
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

fn validate_input(
    input: &CreateCustomProviderInput,
    original_tree: &Value,
    catalog: &BundledCatalog,
) -> Result<ValidatedCreate, AppError> {
    let provider_id = normalize_provider_id(&input.provider.id)?;
    let model_id = normalize_model_id(&input.first_model.id)?;
    let providers = providers_mapping(original_tree)?;
    validate_existing_provider_ids(providers)?;
    if providers
        .keys()
        .filter_map(value_string)
        .any(|id| id.eq_ignore_ascii_case(&provider_id))
    {
        return Err(AppError::new(
            "provider-id-conflict",
            "Provider ID 与现有 Provider 冲突。",
            "请选择一个不区分大小写也唯一的 Provider ID。",
        ));
    }
    if catalog.contains_provider(&provider_id) {
        return Err(AppError::new(
            "provider-id-conflict",
            "Provider ID 与 OMP 内置 Provider 冲突。",
            "请选择一个不区分大小写也不与 bundled catalog 冲突的 Provider ID。",
        ));
    }

    let base_url = normalize_base_url(&input.provider.base_url)?;
    let api_key = validate_api_key(input.provider.auth_mode, input.provider.api_key.as_deref())?;
    if input.first_model.name.trim().is_empty() {
        return Err(AppError::new(
            "model-name-required",
            "Model 名称不能为空。",
            "请填写首个 Model definition 的名称。",
        ));
    }
    if input.first_model.input.is_empty() {
        return Err(AppError::new(
            "model-input-required",
            "Model 至少需要支持 Text 或 Image 一种输入。",
            "请选择 Text、Image 或两者。",
        ));
    }
    if input.first_model.context_window == 0 {
        return Err(AppError::new(
            "model-context-window-invalid",
            "Context Window 必须是正整数。",
            "请输入大于 0 的 Context Window。",
        ));
    }
    if input.first_model.max_tokens == 0 {
        return Err(AppError::new(
            "model-token-limit-invalid",
            "Max Tokens 必须是正整数。",
            "请输入大于 0 的 Max Tokens。",
        ));
    }
    if input
        .first_model
        .api
        .or(input.provider.default_api)
        .is_none()
    {
        return Err(AppError::new(
            "model-api-required",
            "Model 必须从 Provider 默认协议或模型协议覆盖获得有效协议。",
            "请选择 Provider 默认协议，或为该模型选择协议覆盖。",
        ));
    }

    let mut provider = Mapping::new();
    provider.insert(Value::String("baseUrl".to_owned()), Value::String(base_url));
    if let Some(api) = input.provider.default_api {
        provider.insert(
            Value::String("api".to_owned()),
            Value::String(api.as_str().to_owned()),
        );
    }
    if input.provider.auth_mode == ProviderAuthMode::ApiKey {
        provider.insert(
            Value::String("apiKey".to_owned()),
            Value::String(api_key.unwrap_or_default()),
        );
    }
    let mut model = Mapping::new();
    model.insert(
        Value::String("id".to_owned()),
        Value::String(model_id.clone()),
    );
    model.insert(
        Value::String("name".to_owned()),
        Value::String(input.first_model.name.clone()),
    );
    if let Some(api) = input.first_model.api {
        model.insert(
            Value::String("api".to_owned()),
            Value::String(api.as_str().to_owned()),
        );
    }
    model.insert(
        Value::String("reasoning".to_owned()),
        Value::Bool(input.first_model.reasoning),
    );
    model.insert(
        Value::String("input".to_owned()),
        Value::Sequence(
            input
                .first_model
                .input
                .iter()
                .map(|input| Value::String(input.as_str().to_owned()))
                .collect(),
        ),
    );
    model.insert(
        Value::String("contextWindow".to_owned()),
        Value::Number(input.first_model.context_window.into()),
    );
    model.insert(
        Value::String("maxTokens".to_owned()),
        Value::Number(input.first_model.max_tokens.into()),
    );
    provider.insert(
        Value::String("models".to_owned()),
        Value::Sequence(vec![Value::Mapping(model)]),
    );

    Ok(ValidatedCreate {
        provider_id,
        model_id,
        provider_value: Value::Mapping(provider),
    })
}

fn validate_edit_input(
    input: &EditCustomProviderInput,
    original_tree: &Value,
    catalog: &BundledCatalog,
) -> Result<ValidatedEdit, AppError> {
    if input.provider_id.is_empty() || input.provider_id.trim() != input.provider_id.as_str() {
        return Err(AppError::new(
            "provider-id-immutable",
            "已有 Provider ID 是 Stable ID，不能修改。",
            "请保留当前 Provider ID；如需新 ID，请创建新的 Provider 后处理引用。",
        ));
    }
    let providers = providers_mapping(original_tree)?;
    validate_existing_provider_ids(providers)?;
    let Some(provider_value) = providers.get(Value::String(input.provider_id.clone())) else {
        if providers
            .keys()
            .filter_map(value_string)
            .any(|id| id.eq_ignore_ascii_case(&input.provider_id))
        {
            return Err(AppError::new(
                "provider-id-immutable",
                "已有 Provider ID 是 Stable ID，不能修改大小写。",
                "请保留当前 Provider ID；如需新 ID，请创建新的 Provider 后处理引用。",
            ));
        }
        return Err(AppError::new(
            "provider-edit-not-found",
            "要编辑的 Provider 已不存在。",
            "请重新读取配置后选择当前存在的 Provider。",
        ));
    };
    let Some(provider) = provider_value.as_mapping() else {
        return Err(AppError::new(
            "provider-edit-unavailable",
            "当前 Provider 配置不能安全编辑。",
            "请重新读取配置并处理不支持的 Provider 结构。",
        ));
    };
    if !crate::overview::is_editable_custom_provider(original_tree, &input.provider_id, catalog) {
        return Err(AppError::new(
            "provider-edit-unavailable",
            "当前 Provider 不是可编辑的 Custom Provider。",
            "内置覆盖、高级配置和不支持的 Provider 保持只读。",
        ));
    }
    let base_url = normalize_base_url(&input.base_url)?;
    validate_provider_default_api(provider, input.default_api)?;
    let api_key = validate_edit_api_key(input.auth_mode, &input.api_key, provider)?;
    Ok(ValidatedEdit {
        provider_id: input.provider_id.clone(),
        base_url,
        default_api: input.default_api,
        api_key,
    })
}

fn validate_provider_default_api(
    provider: &Mapping,
    default_api: Option<SupportedApi>,
) -> Result<(), AppError> {
    if default_api.is_some() {
        return Ok(());
    }
    let Some(models) = provider
        .get(Value::String("models".to_owned()))
        .and_then(Value::as_sequence)
    else {
        return Err(AppError::new(
            "provider-edit-unavailable",
            "当前 Provider 的 Model definition 结构不能安全编辑。",
            "请重新读取配置并处理不支持的 Provider 结构。",
        ));
    };
    let all_models_have_api = models.iter().all(|model| {
        model
            .as_mapping()
            .and_then(|model| model.get(Value::String("api".to_owned())))
            .and_then(Value::as_str)
            .is_some_and(|api| {
                matches!(
                    api,
                    "openai-completions"
                        | "openai-responses"
                        | "anthropic-messages"
                        | "google-generative-ai"
                )
            })
    });
    if all_models_have_api {
        return Ok(());
    }
    Err(AppError::new(
        "provider-default-api-required",
        "移除默认协议会让部分 Model definition 没有有效协议。",
        "请保留默认协议，或先为每个 Model definition 设置支持的协议覆盖。",
    ))
}

fn validate_direct_api_key_replacement(value: &str) -> Result<String, AppError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || is_masked_direct_api_key(trimmed) || trimmed.starts_with('!') {
        return Err(AppError::new(
            "provider-api-key-invalid",
            "Direct API Key 替换值必须是非空的直接文本，且不能是掩码或命令凭据。",
            "请填写新的 Direct API Key，或选择保留当前密钥。",
        ));
    }
    Ok(value.to_owned())
}

fn validate_edit_api_key(
    auth_mode: ProviderAuthMode,
    intent: &DirectApiKeyIntent,
    provider: &Mapping,
) -> Result<ValidatedApiKeyIntent, AppError> {
    let existing = provider.get(Value::String("apiKey".to_owned()));
    let has_direct_key_field = matches!(existing, Some(Value::String(_)));
    match (auth_mode, intent) {
        (ProviderAuthMode::ApiKey, DirectApiKeyIntent::Keep) if has_direct_key_field => {
            Ok(ValidatedApiKeyIntent::Keep)
        }
        (ProviderAuthMode::ApiKey, DirectApiKeyIntent::Replace { value }) => Ok(
            ValidatedApiKeyIntent::Replace(validate_direct_api_key_replacement(value)?),
        ),
        (ProviderAuthMode::None, DirectApiKeyIntent::Delete) => Ok(ValidatedApiKeyIntent::Delete),
        (ProviderAuthMode::None, DirectApiKeyIntent::Keep) if !has_direct_key_field => {
            Ok(ValidatedApiKeyIntent::Keep)
        }
        (ProviderAuthMode::ApiKey, DirectApiKeyIntent::Keep) => Err(AppError::new(
            "provider-auth-invalid",
            "API Key 认证需要保留现有 Direct API Key 或输入新的替换值。",
            "请选择无认证，或输入新的 Direct API Key。",
        )),
        (ProviderAuthMode::ApiKey, DirectApiKeyIntent::Delete) => Err(AppError::new(
            "provider-auth-invalid",
            "删除 Direct API Key 时必须切换为无需认证。",
            "请确认切换为无需认证后再删除密钥。",
        )),
        (ProviderAuthMode::None, DirectApiKeyIntent::Keep) => Err(AppError::new(
            "provider-auth-invalid",
            "切换为无需认证时必须明确删除现有 Direct API Key。",
            "请确认删除密钥，或继续使用 API Key 认证。",
        )),
        (ProviderAuthMode::None, DirectApiKeyIntent::Replace { .. }) => Err(AppError::new(
            "provider-auth-invalid",
            "无需认证 Provider 不能同时替换 Direct API Key。",
            "请选择 API Key 认证后再输入新的 Direct API Key。",
        )),
    }
}

fn is_masked_direct_api_key(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value
            .chars()
            .all(|character| matches!(character, '*' | '•' | '●' | '▪' | '█' | 'x' | 'X'))
}

fn normalize_provider_id(value: &str) -> Result<String, AppError> {
    let normalized = value.trim();
    let mut characters = normalized.bytes();
    let Some(first) = characters.next() else {
        return Err(provider_id_error());
    };
    if !first.is_ascii_alphanumeric()
        || !characters.all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, b'.' | b'_' | b'-')
        })
    {
        return Err(provider_id_error());
    }
    Ok(normalized.to_owned())
}

fn provider_id_error() -> AppError {
    AppError::new(
        "provider-id-invalid",
        "Provider ID 必须匹配 [A-Za-z0-9][A-Za-z0-9._-]*。",
        "请移除空白、/、: 和其他不支持的字符。",
    )
}
pub(crate) fn normalize_model_id(value: &str) -> Result<String, AppError> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
    {
        return Err(AppError::new(
            "model-id-invalid",
            "Model ID 必须非空且不含空白或控制字符。",
            "请使用不含空白或控制字符的 Model ID。",
        ));
    }
    Ok(normalized.to_owned())
}

fn normalize_base_url(value: &str) -> Result<String, AppError> {
    let normalized = value.trim().trim_end_matches('/');
    let valid = Url::parse(normalized)
        .ok()
        .is_some_and(|url| matches!(url.scheme(), "http" | "https") && url.host_str().is_some());
    if !valid {
        return Err(AppError::new(
            "provider-base-url-invalid",
            "Base URL 必须是有效的 HTTP 或 HTTPS 地址。",
            "请填写完整地址；OMP Switch 不会自动补充 API 版本路径。",
        ));
    }
    Ok(normalized.to_owned())
}

fn validate_api_key(
    auth_mode: ProviderAuthMode,
    api_key: Option<&str>,
) -> Result<Option<String>, AppError> {
    let has_key = api_key.is_some_and(|key| !key.is_empty());
    if auth_mode == ProviderAuthMode::None && has_key {
        return Err(AppError::new(
            "provider-auth-invalid",
            "无认证 Provider 不能同时提交 API Key。",
            "请选择 API Key 认证，或清除 API Key 后继续。",
        ));
    }
    if auth_mode == ProviderAuthMode::ApiKey && api_key.is_some_and(|key| key.starts_with('!')) {
        return Err(AppError::new(
            "provider-api-key-invalid",
            "Direct API Key 不能以 ! 开头。",
            "请填写直接文本 API Key，不要使用命令凭据。",
        ));
    }
    Ok((auth_mode == ProviderAuthMode::ApiKey && has_key)
        .then(|| api_key.expect("checked API Key presence").to_owned()))
}

fn providers_mapping(tree: &Value) -> Result<&Mapping, AppError> {
    let Value::Mapping(root) = tree else {
        return Err(models_structure_error());
    };
    root.get(Value::String("providers".to_owned()))
        .and_then(Value::as_mapping)
        .ok_or_else(models_structure_error)
}

fn validate_existing_provider_ids(providers: &Mapping) -> Result<(), AppError> {
    let mut identifiers = HashSet::with_capacity(providers.len());
    for key in providers.keys() {
        let Some(identifier) = value_string(key) else {
            return Err(models_structure_error());
        };
        if !identifiers.insert(identifier.to_ascii_lowercase()) {
            return Err(models_structure_error());
        }
    }
    Ok(())
}

fn models_structure_error() -> AppError {
    AppError::new(
        "models-structure-invalid",
        "models.yml 的 providers 结构不适合安全创建 Provider。",
        "请在外部修复 Provider 结构后重新读取；OMP Switch 不会覆盖该文件。",
    )
}

fn value_string(value: &Value) -> Option<&str> {
    match value {
        Value::String(value) => Some(value),
        _ => None,
    }
}

fn insert_provider(
    tree: &mut Value,
    provider_id: &str,
    provider_value: Value,
) -> Result<(), AppError> {
    let Value::Mapping(root) = tree else {
        return Err(models_structure_error());
    };
    let Some(Value::Mapping(providers)) = root.get_mut(Value::String("providers".to_owned()))
    else {
        return Err(models_structure_error());
    };
    if providers
        .insert(Value::String(provider_id.to_owned()), provider_value)
        .is_some()
    {
        return Err(AppError::new(
            "provider-id-conflict",
            "Provider ID 与现有 Provider 冲突。",
            "请选择一个不区分大小写也唯一的 Provider ID。",
        ));
    }
    Ok(())
}

fn validate_created_provider(tree: &Value, validated: &ValidatedCreate) -> Result<(), AppError> {
    let providers = providers_mapping(tree)?;
    let created = providers
        .get(Value::String(validated.provider_id.clone()))
        .ok_or_else(|| {
            AppError::new(
                "models-temporary-validation-error",
                "临时 models.yml 未包含完整的新 Provider。",
                "请重试；原文件没有被修改。",
            )
        })?;
    if created != &validated.provider_value {
        return Err(AppError::new(
            "models-temporary-validation-error",
            "临时 models.yml 未能保留新 Provider 的已验证字段。",
            "请重试；原文件没有被修改。",
        ));
    }
    Ok(())
}

fn update_provider(tree: &mut Value, validated: &ValidatedEdit) -> Result<(), AppError> {
    let Value::Mapping(root) = tree else {
        return Err(models_structure_error());
    };
    let Some(Value::Mapping(providers)) = root.get_mut(Value::String("providers".to_owned()))
    else {
        return Err(models_structure_error());
    };
    let Some(Value::Mapping(provider)) =
        providers.get_mut(Value::String(validated.provider_id.clone()))
    else {
        return Err(AppError::new(
            "models-temporary-validation-error",
            "临时 models.yml 未包含要编辑的 Provider。",
            "请重新读取配置后重试；原文件没有被修改。",
        ));
    };
    provider.insert(
        Value::String("baseUrl".to_owned()),
        Value::String(validated.base_url.clone()),
    );
    match validated.default_api {
        Some(api) => {
            provider.insert(
                Value::String("api".to_owned()),
                Value::String(api.as_str().to_owned()),
            );
        }
        None => {
            provider.remove(Value::String("api".to_owned()));
        }
    }
    match &validated.api_key {
        ValidatedApiKeyIntent::Keep => {}
        ValidatedApiKeyIntent::Replace(value) => {
            provider.insert(
                Value::String("apiKey".to_owned()),
                Value::String(value.clone()),
            );
        }
        ValidatedApiKeyIntent::Delete => {
            provider.remove(Value::String("apiKey".to_owned()));
        }
    }
    Ok(())
}

fn validate_edited_provider(
    candidate: &Value,
    original: &Value,
    validated: &ValidatedEdit,
) -> Result<(), AppError> {
    let candidate_provider = providers_mapping(candidate)?
        .get(Value::String(validated.provider_id.clone()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| {
            AppError::new(
                "models-temporary-validation-error",
                "临时 models.yml 未包含可验证的 Provider 编辑结果。",
                "请重试；原文件没有被修改。",
            )
        })?;
    if candidate_provider
        .get(Value::String("baseUrl".to_owned()))
        .and_then(Value::as_str)
        != Some(validated.base_url.as_str())
    {
        return Err(AppError::new(
            "models-temporary-validation-error",
            "临时 models.yml 未保留已验证的 Base URL。",
            "请重试；原文件没有被修改。",
        ));
    }
    let has_expected_api = match validated.default_api {
        Some(api) => {
            candidate_provider
                .get(Value::String("api".to_owned()))
                .and_then(Value::as_str)
                == Some(api.as_str())
        }
        None => !candidate_provider.contains_key(Value::String("api".to_owned())),
    };
    if !has_expected_api {
        return Err(AppError::new(
            "models-temporary-validation-error",
            "临时 models.yml 未保留已验证的默认协议。",
            "请重试；原文件没有被修改。",
        ));
    }
    let original_key = providers_mapping(original)?
        .get(Value::String(validated.provider_id.clone()))
        .and_then(Value::as_mapping)
        .and_then(|provider| provider.get(Value::String("apiKey".to_owned())));
    let has_expected_key = match &validated.api_key {
        ValidatedApiKeyIntent::Keep => {
            candidate_provider.get(Value::String("apiKey".to_owned())) == original_key
        }
        ValidatedApiKeyIntent::Replace(value) => {
            candidate_provider
                .get(Value::String("apiKey".to_owned()))
                .and_then(Value::as_str)
                == Some(value.as_str())
        }
        ValidatedApiKeyIntent::Delete => {
            !candidate_provider.contains_key(Value::String("apiKey".to_owned()))
        }
    };
    if !has_expected_key {
        return Err(AppError::new(
            "models-temporary-validation-error",
            "临时 models.yml 未保留已验证的 Direct API Key 操作。",
            "请重试；原文件没有被修改。",
        ));
    }
    Ok(())
}

fn ensure_edited_untouched_paths_equal(
    original: &Value,
    candidate: &Value,
    provider_id: &str,
    edited_fields: &[&str],
) -> Result<(), AppError> {
    let Value::Mapping(original_root) = original else {
        return Err(models_structure_error());
    };
    let Value::Mapping(candidate_root) = candidate else {
        return Err(untouched_value_error());
    };
    if original_root.len() != candidate_root.len() {
        return Err(untouched_value_error());
    }
    for (key, original_value) in original_root {
        if value_string(key) != Some("providers") && candidate_root.get(key) != Some(original_value)
        {
            return Err(untouched_value_error());
        }
    }
    let original_providers = providers_mapping(original)?;
    let candidate_providers = providers_mapping(candidate).map_err(|_| untouched_value_error())?;
    if original_providers.len() != candidate_providers.len() {
        return Err(untouched_value_error());
    }
    for (key, original_value) in original_providers {
        let Some(candidate_value) = candidate_providers.get(key) else {
            return Err(untouched_value_error());
        };
        if value_string(key) != Some(provider_id) {
            if candidate_value != original_value {
                return Err(untouched_value_error());
            }
            continue;
        }
        ensure_edited_provider_untouched_fields_equal(
            original_value,
            candidate_value,
            edited_fields,
        )?;
    }
    Ok(())
}

fn ensure_edited_provider_untouched_fields_equal(
    original: &Value,
    candidate: &Value,
    edited_fields: &[&str],
) -> Result<(), AppError> {
    let Some(original) = original.as_mapping() else {
        return Err(untouched_value_error());
    };
    let Some(candidate) = candidate.as_mapping() else {
        return Err(untouched_value_error());
    };
    for (key, original_value) in original {
        if edited_fields.contains(&value_string(key).unwrap_or_default()) {
            continue;
        }
        if candidate.get(key) != Some(original_value) {
            return Err(untouched_value_error());
        }
    }
    for (key, candidate_value) in candidate {
        if edited_fields.contains(&value_string(key).unwrap_or_default()) {
            continue;
        }
        if original.get(key) != Some(candidate_value) {
            return Err(untouched_value_error());
        }
    }
    Ok(())
}

fn ensure_untouched_paths_equal(
    original: &Value,
    candidate: &Value,
    created_provider_id: &str,
) -> Result<(), AppError> {
    let Value::Mapping(original_root) = original else {
        return Err(models_structure_error());
    };
    let Value::Mapping(candidate_root) = candidate else {
        return Err(untouched_value_error());
    };
    if original_root.len() != candidate_root.len() {
        return Err(untouched_value_error());
    }
    for (key, original_value) in original_root {
        if value_string(key) == Some("providers") {
            continue;
        }
        if candidate_root.get(key) != Some(original_value) {
            return Err(untouched_value_error());
        }
    }
    let original_providers = providers_mapping(original)?;
    let candidate_providers = providers_mapping(candidate).map_err(|_| untouched_value_error())?;
    if candidate_providers.len() != original_providers.len() + 1 {
        return Err(untouched_value_error());
    }
    for (key, original_value) in original_providers {
        if candidate_providers.get(key) != Some(original_value) {
            return Err(untouched_value_error());
        }
    }
    if !candidate_providers.contains_key(Value::String(created_provider_id.to_owned())) {
        return Err(untouched_value_error());
    }
    Ok(())
}

fn untouched_value_error() -> AppError {
    AppError::new(
        "models-untouched-path-changed",
        "序列化会改变未触及的配置路径；OMP Switch 已停止写入。",
        "请检查配置后重试；原 models.yml 没有被修改。",
    )
}

fn create_current_backup(
    backup_root: &Path,
    resolved_target: &Path,
    original_bytes: &[u8],
    failure: Option<ProviderMutationFailurePoint>,
) -> Result<(), AppError> {
    let target_fingerprint = content_hash(resolved_target.to_string_lossy().as_bytes());
    let target_directory = backup_root.join(target_fingerprint);
    let directory = target_directory.join("models.yml");
    for path in [backup_root, target_directory.as_path(), directory.as_path()] {
        create_private_backup_directory(path, failure)
            .map_err(|error| write_error("create_backup_directory", error))?;
    }
    let (backup_path, mut backup) = allocate_backup_file(&directory, failure)
        .map_err(|error| write_error("create_models_backup", error))?;
    let result = (|| -> std::io::Result<()> {
        #[cfg(test)]
        inject_io_failure(
            failure,
            ProviderMutationFailurePoint::BackupFileWriteFailure,
        )?;
        backup.write_all(original_bytes)?;
        #[cfg(test)]
        inject_io_failure(failure, ProviderMutationFailurePoint::BackupFileSyncFailure)?;
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
        Ok(()) => Ok(()),
        Err(error) => {
            cleanup_partial_backup(&backup_path);
            Err(write_error("create_models_backup", error))
        }
    }
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
    failure: Option<ProviderMutationFailurePoint>,
) -> std::io::Result<()> {
    #[cfg(test)]
    inject_io_failure(
        failure,
        ProviderMutationFailurePoint::BackupDirectoryCreationFailure,
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
    failure: Option<ProviderMutationFailurePoint>,
) -> std::io::Result<(PathBuf, fs::File)> {
    for _ in 0..BACKUP_ALLOCATION_ATTEMPTS {
        let backup_path =
            backup_candidate_path(directory, BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed));
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        #[cfg(test)]
        inject_io_failure(failure, ProviderMutationFailurePoint::BackupFileOpenFailure)?;
        match options.open(&backup_path) {
            Ok(backup) => {
                #[cfg(all(test, unix))]
                let _restore_backup_directory_permissions = if failure
                    == Some(ProviderMutationFailurePoint::BackupFilePermissionAndCleanupFailure)
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

fn set_private_backup_file_permissions(
    path: &Path,
    failure: Option<ProviderMutationFailurePoint>,
) -> std::io::Result<()> {
    #[cfg(all(test, unix))]
    if failure == Some(ProviderMutationFailurePoint::BackupFilePermissionAndCleanupFailure) {
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
    failure: Option<ProviderMutationFailurePoint>,
    point: ProviderMutationFailurePoint,
) -> std::io::Result<()> {
    if failure == Some(point) {
        return Err(std::io::Error::other(
            "injected provider creation I/O failure",
        ));
    }
    Ok(())
}

fn backup_candidate_path(directory: &Path, sequence: u64) -> PathBuf {
    directory.join(format!("{}-{sequence}.yml", std::process::id()))
}

pub(crate) fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn remap_edit_operation_error(error: AppError) -> AppError {
    let code = match error.code {
        "provider-create-unavailable" => Some("provider-edit-unavailable"),
        "provider-create-target-changed" => Some("provider-edit-target-changed"),
        "provider-create-failed"
        | "models-serialize-error"
        | "models-temporary-parse-error"
        | "models-temporary-validation-error"
        | "models-untouched-path-changed" => Some("provider-edit-failed"),
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
        .find(|(key, _)| value_string(key) != Some("providers"))
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
        let target = temporary.path().join("agent/models.yml");
        let original = b"original models";
        let directory = backup_root
            .join(content_hash(target.to_string_lossy().as_bytes()))
            .join("models.yml");
        fs::create_dir_all(&directory).unwrap();
        let first_candidate = directory.join(format!("{}-0.yml", std::process::id()));
        fs::write(&first_candidate, b"existing backup").unwrap();
        BACKUP_SEQUENCE.store(0, Ordering::Relaxed);

        create_current_backup(&backup_root, &target, original, None).unwrap();

        assert_eq!(fs::read(first_candidate).unwrap(), b"existing backup");
        assert_eq!(
            fs::read(directory.join(format!("{}-1.yml", std::process::id()))).unwrap(),
            original
        );
    }
}
