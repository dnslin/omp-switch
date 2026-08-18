use std::{collections::HashSet, path::Path};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use url::Url;

use crate::{
    bundled_catalog::BundledCatalog,
    error::AppError,
    models_write::{self, ModelsMutation, ModelsWriteFailurePoint},
    target_configuration::TargetConfigurationDiscovery,
};

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

pub(crate) fn create_custom_provider(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &CreateCustomProviderInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<CreateCustomProviderResult, AppError> {
    let loaded = models_write::load_models_for_write(target, &input.opened_models_hash)?;
    let validated = validate_input(input, &loaded.original_tree, catalog)?;
    let result = CreateCustomProviderResult {
        provider_id: validated.provider_id.clone(),
        model_id: validated.model_id.clone(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)?;
    Ok(result)
}

pub(crate) fn edit_custom_provider(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &EditCustomProviderInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<EditCustomProviderResult, AppError> {
    let loaded = models_write::load_models_for_write(target, &input.opened_models_hash)
        .map_err(remap_edit_operation_error)?;
    let validated = validate_edit_input(input, &loaded.original_tree, catalog)?;
    let result = EditCustomProviderResult {
        provider_id: validated.provider_id.clone(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(remap_edit_operation_error)?;
    Ok(result)
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

fn remap_edit_operation_error(error: AppError) -> AppError {
    models_write::remap_models_write_error(
        error,
        models_write::ModelsWriteErrorCodes {
            unavailable: "provider-edit-unavailable",
            target_changed: "provider-edit-target-changed",
            failed: "provider-edit-failed",
        },
    )
}
