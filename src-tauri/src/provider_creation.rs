use std::{
    collections::HashSet,
    fs,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

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
    fn as_str(self) -> &'static str {
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
    fn as_str(self) -> &'static str {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ProviderCreationFailurePoint {
    BeforeBackup,
    AfterBackup,
    BeforeTemporaryWrite,
    CorruptTemporaryFile,
    MutateUntouchedValue,
    BeforeReplacement,
    CommitFailureAfterReplacement,
    AfterReplacement,
}

struct ValidatedCreate {
    provider_id: String,
    model_id: String,
    provider_value: Value,
}

pub(crate) fn create_custom_provider(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &CreateCustomProviderInput,
    failure: Option<ProviderCreationFailurePoint>,
) -> Result<CreateCustomProviderResult, AppError> {
    validate_writable_target(target)?;
    let expected_target = resolved_path(&target.resolved_path, "Target configuration")?;
    let models_path = resolved_path(&target.models.resolved_path, "models.yml")?;
    ensure_resolved_models_path(&models_path, &expected_target)?;

    let original_bytes =
        fs::read(&models_path).map_err(|error| write_error("read_models", error))?;
    let original_hash = content_hash(&original_bytes);
    if original_hash != input.opened_models_hash {
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
    let validated = validate_input(input, &original_tree, catalog)?;

    if failure == Some(ProviderCreationFailurePoint::BeforeBackup) {
        return Err(injected_failure("创建当前备份前发生故障。"));
    }
    create_current_backup(backup_root, &expected_target, &original_bytes)?;
    if failure == Some(ProviderCreationFailurePoint::AfterBackup) {
        return Err(injected_failure("创建当前备份后发生故障。"));
    }

    let mut candidate = original_tree.clone();
    insert_provider(
        &mut candidate,
        &validated.provider_id,
        validated.provider_value.clone(),
    )?;
    let serialized = serde_yaml::to_string(&candidate).map_err(|error| {
        yaml_error(
            "serialize_created_provider",
            "models-serialize-error",
            "无法序列化新的 Provider 配置",
            "请检查表单后重试；原 models.yml 没有被修改。",
            error,
        )
    })?;

    if failure == Some(ProviderCreationFailurePoint::BeforeTemporaryWrite) {
        return Err(injected_failure("写入临时文件前发生故障。"));
    }
    let mut temporary = AtomicWriteFile::options()
        .read(true)
        .open(&models_path)
        .map_err(|error| write_error("open_models_temporary", error))?;
    temporary
        .write_all(serialized.as_bytes())
        .and_then(|()| temporary.sync_all())
        .map_err(|error| write_error("write_models_temporary", error))?;
    if failure == Some(ProviderCreationFailurePoint::CorruptTemporaryFile) {
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
    if failure == Some(ProviderCreationFailurePoint::MutateUntouchedValue) {
        mutate_untouched_value_for_test(&mut reparsed);
    }
    validate_created_provider(&reparsed, &validated)?;
    ensure_untouched_paths_equal(&original_tree, &reparsed, &validated.provider_id)?;

    ensure_resolved_models_path(&models_path, &expected_target)?;
    let latest_bytes =
        fs::read(&models_path).map_err(|error| write_error("recheck_models", error))?;
    if content_hash(&latest_bytes) != original_hash {
        return Err(hash_conflict());
    }
    if failure == Some(ProviderCreationFailurePoint::BeforeReplacement) {
        return Err(injected_failure("原子替换前发生故障。"));
    }
    let replacement = temporary
        .commit()
        .map_err(|error| write_error("replace_models", error))
        .and_then(|()| {
            if failure == Some(ProviderCreationFailurePoint::CommitFailureAfterReplacement) {
                Err(injected_failure("原子替换在提交后报告故障。"))
            } else {
                Ok(())
            }
        });
    if let Err(error) = replacement {
        return Err(restore_after_postwrite_failure(
            &models_path,
            &expected_target,
            serialized.as_bytes(),
            &original_bytes,
            error,
        ));
    }

    let postwrite = (|| -> Result<(), AppError> {
        if failure == Some(ProviderCreationFailurePoint::AfterReplacement) {
            return Err(injected_failure("原子替换后发生故障。"));
        }
        let committed_bytes =
            fs::read(&models_path).map_err(|error| write_error("reread_models", error))?;
        let committed = serde_yaml::from_slice::<Value>(&committed_bytes).map_err(|error| {
            yaml_error(
                "parse_committed_models",
                "models-postwrite-parse-error",
                "写入后的 models.yml 无法重新解析",
                "OMP Switch 将恢复原文件；请检查 YAML 后重试。",
                error,
            )
        })?;
        validate_created_provider(&committed, &validated)?;
        ensure_untouched_paths_equal(&original_tree, &committed, &validated.provider_id)
    })();
    if let Err(error) = postwrite {
        return Err(restore_after_postwrite_failure(
            &models_path,
            &expected_target,
            serialized.as_bytes(),
            &original_bytes,
            error,
        ));
    }

    Ok(CreateCustomProviderResult {
        provider_id: validated.provider_id,
        model_id: validated.model_id,
    })
}

fn restore_after_postwrite_failure(
    models_path: &Path,
    expected_target: &Path,
    candidate_bytes: &[u8],
    original_bytes: &[u8],
    postwrite_error: AppError,
) -> AppError {
    match restore_original_models(
        models_path,
        expected_target,
        candidate_bytes,
        original_bytes,
    ) {
        Ok(()) => postwrite_error,
        Err(recovery_error) => {
            tracing::error!(
                operation = "restore_models_after_postwrite_failure",
                postwrite_code = postwrite_error.code,
                recovery_code = recovery_error.code,
                "Provider creation postwrite recovery failed"
            );
            recovery_error
        }
    }
}

fn restore_original_models(
    models_path: &Path,
    expected_target: &Path,
    candidate_bytes: &[u8],
    original_bytes: &[u8],
) -> Result<(), AppError> {
    if let Err(error) = ensure_resolved_models_path(models_path, expected_target) {
        tracing::error!(
            operation = "verify_models_path_before_recovery",
            code = error.code,
            "Provider creation recovery target changed"
        );
        return Err(postwrite_recovery_error());
    }
    let current = fs::read(models_path)
        .map_err(|error| postwrite_recovery_io_error("read_models_for_recovery", error))?;
    if current == original_bytes {
        return Ok(());
    }
    if current != candidate_bytes {
        tracing::error!(
            operation = "verify_models_before_recovery",
            "Provider creation recovery refuses to overwrite changed models.yml"
        );
        return Err(postwrite_recovery_error());
    }

    let mut recovery = AtomicWriteFile::options()
        .read(true)
        .open(models_path)
        .map_err(|error| postwrite_recovery_io_error("open_models_recovery", error))?;
    recovery
        .write_all(original_bytes)
        .and_then(|()| recovery.sync_all())
        .map_err(|error| postwrite_recovery_io_error("write_models_recovery", error))?;
    recovery
        .commit()
        .map_err(|error| postwrite_recovery_io_error("replace_models_recovery", error))?;
    let restored = fs::read(models_path)
        .map_err(|error| postwrite_recovery_io_error("verify_models_recovery", error))?;
    if restored != original_bytes {
        tracing::error!(
            operation = "verify_models_recovery",
            "Provider creation recovery did not restore the original models.yml"
        );
        return Err(postwrite_recovery_error());
    }
    Ok(())
}

fn postwrite_recovery_io_error(operation: &'static str, error: std::io::Error) -> AppError {
    tracing::error!(
        operation,
        cause = io_error_cause(error.kind()),
        "Provider creation rollback failed"
    );
    postwrite_recovery_error()
}

fn postwrite_recovery_error() -> AppError {
    AppError::new(
        "models-postwrite-recovery-failed",
        "写入后验证失败，且 OMP Switch 无法安全恢复原 models.yml。",
        "请立即停止写入，并从 Target configuration 的当前备份恢复 models.yml。",
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

fn validate_writable_target(target: &TargetConfigurationDiscovery) -> Result<(), AppError> {
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

fn resolved_path(path: &Option<String>, label: &str) -> Result<PathBuf, AppError> {
    path.as_deref().map(PathBuf::from).ok_or_else(|| {
        AppError::new(
            "provider-create-unavailable",
            format!("无法确认 {label} 的真实路径。"),
            "请重新检测 OMP 并修复链接、路径类型或权限问题。",
        )
    })
}

fn ensure_resolved_models_path(models_path: &Path, expected_target: &Path) -> Result<(), AppError> {
    let resolved_target = expected_target
        .canonicalize()
        .map_err(|error| write_error("resolve_target", error))?;
    let resolved_models = models_path
        .canonicalize()
        .map_err(|error| write_error("resolve_models", error))?;
    if resolved_target != expected_target || resolved_models != models_path {
        return Err(AppError::new(
            "provider-create-target-changed",
            "Target configuration 的真实文件目标已变化。",
            "请重新检测 OMP；OMP Switch 不会向变化后的路径写入。",
        ));
    }
    if !fs::metadata(&resolved_models)
        .map_err(|error| write_error("inspect_models", error))?
        .is_file()
    {
        return Err(AppError::new(
            "provider-create-target-changed",
            "models.yml 的真实目标不是普通文件。",
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
    if catalog.contains_model(&provider_id, &model_id) {
        return Err(AppError::new(
            "model-id-conflict",
            "Model ID 与 OMP 内置 Model definition 冲突。",
            "请选择一个不与 bundled catalog 冲突的 Model ID。",
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
    if input.first_model.max_tokens == 0
        || input.first_model.max_tokens > input.first_model.context_window
    {
        return Err(AppError::new(
            "model-token-limit-invalid",
            "Max Tokens 必须是正整数且不能超过 Context Window。",
            "请调整 Max Tokens 或 Context Window。",
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
    if let Some(api_key) = api_key {
        provider.insert(Value::String("apiKey".to_owned()), Value::String(api_key));
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

fn normalize_model_id(value: &str) -> Result<String, AppError> {
    let normalized = value.trim();
    if normalized.is_empty()
        || normalized
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        || has_thinking_suffix(normalized)
    {
        return Err(AppError::new(
            "model-id-invalid",
            "Model ID 必须非空、不含空白或控制字符，且不能使用 Thinking Level 后缀。",
            "请使用不与角色 Thinking Level 歧义的 Model ID。",
        ));
    }
    Ok(normalized.to_owned())
}

fn has_thinking_suffix(value: &str) -> bool {
    let suffix = value.rsplit_once(':').map(|(_, suffix)| suffix);
    matches!(
        suffix.map(|suffix| suffix.to_ascii_lowercase()),
        Some(suffix)
            if matches!(
                suffix.as_str(),
                "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto"
            )
    )
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
) -> Result<(), AppError> {
    let target_fingerprint = content_hash(resolved_target.to_string_lossy().as_bytes());
    let directory = backup_root.join(target_fingerprint).join("models.yml");
    fs::create_dir_all(&directory)
        .map_err(|error| write_error("create_backup_directory", error))?;
    let sequence = BACKUP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let backup_path = directory.join(format!("{}-{sequence}.yml", std::process::id(),));
    let result = (|| -> std::io::Result<()> {
        let mut backup = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&backup_path)?;
        backup.write_all(original_bytes)?;
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
            if let Err(cleanup_error) = fs::remove_file(&backup_path)
                && cleanup_error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(
                    operation = "cleanup_partial_models_backup",
                    cause = io_error_cause(cleanup_error.kind()),
                    "Provider creation backup cleanup failed"
                );
            }
            Err(write_error("create_models_backup", error))
        }
    }
}

fn content_hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn hash_conflict() -> AppError {
    AppError::new(
        "models-hash-conflict",
        "models.yml 在打开表单后已被外部修改。",
        "请重新读取配置；当前表单输入已保留，OMP Switch 不会自动合并。",
    )
}

fn injected_failure(message: &'static str) -> AppError {
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
