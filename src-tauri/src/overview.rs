use std::{collections::HashSet, fs};

use serde::{Serialize, Serializer};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::{
    error::AppError,
    redaction::{redact_diagnostic, redact_projection},
    target_configuration::{TargetConfigurationDiscovery, TargetConfigurationStatus},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OverviewState {
    Normal,
    Empty,
    ReadOnly,
}

impl Serialize for OverviewState {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match self {
            Self::Normal => "normal",
            Self::Empty => "empty",
            Self::ReadOnly => "read-only",
        })
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewDto {
    pub state: OverviewState,
    pub omp: OmpOverviewDto,
    pub target_configuration: TargetConfigurationDiscovery,
    pub files: OverviewFilesDto,
    pub counts: OverviewCountsDto,
    pub providers: Vec<ProviderSummaryDto>,
    pub models: Vec<ModelSummaryDto>,
    pub roles: Vec<RoleSummaryDto>,
    pub empty_reason: Option<String>,
    pub next_action: Option<String>,
    pub read_only_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OmpOverviewDto {
    pub status: &'static str,
    pub executable_path: String,
    pub version: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewFilesDto {
    pub models: OverviewFileDto,
    pub config: OverviewFileDto,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewFileDto {
    pub canonical_path: String,
    pub resolved_path: Option<String>,
    pub status: crate::target_configuration::ConfigurationFileStatus,
    pub content_hash: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewCountsDto {
    pub provider_count: usize,
    pub model_count: usize,
    pub role_count: usize,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderSummaryDto {
    pub id: String,
    pub name: Option<String>,
    pub base_url: Option<String>,
    pub default_api: Option<String>,
    pub auth_mode: String,
    pub has_api_key: bool,
    pub model_count: usize,
    pub editable: bool,
    pub read_only_reason: Option<String>,
    pub models: Vec<ModelSummaryDto>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummaryDto {
    pub provider_id: String,
    pub id: String,
    pub name: Option<String>,
    pub effective_api: Option<String>,
    pub api_source: Option<String>,
    pub input: Vec<String>,
    pub reasoning: Option<bool>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub complete: bool,
    pub editable: bool,
    pub read_only_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSummaryDto {
    pub id: String,
    pub status: String,
    pub selector: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ParsedConfiguration {
    pub(crate) raw_hash: String,
    pub(crate) tree: Value,
}

#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct ConfigurationSnapshot {
    pub(crate) models: ParsedConfiguration,
    pub(crate) config: ParsedConfiguration,
}

pub(crate) struct OverviewReadResult {
    pub(crate) dto: OverviewDto,
    pub(crate) snapshot: Option<ConfigurationSnapshot>,
}

pub(crate) fn read_overview(
    executable_path: &str,
    version: &str,
    target: &TargetConfigurationDiscovery,
) -> Result<OverviewReadResult, AppError> {
    match target.status {
        TargetConfigurationStatus::Unsafe => {
            return Err(AppError::new(
                "overview-unsafe-target",
                "无法安全读取 Target configuration。",
                "请修复链接、路径类型或权限边界后重新读取。",
            ));
        }
        TargetConfigurationStatus::MigrationRequired => {
            return Err(AppError::new(
                "overview-migration-required",
                "当前配置需要先由 OMP 完成官方 YAML 迁移。",
                "请在 OMP 中完成迁移后重新读取。",
            ));
        }
        TargetConfigurationStatus::ParseError => {
            let detail = target
                .issue
                .as_ref()
                .map(|issue| format!("{}：{}", issue.file_path, redact_diagnostic(&issue.message)))
                .unwrap_or_else(|| "YAML 存在格式错误。".to_owned());
            return Err(AppError::new(
                "overview-parse-error",
                format!("无法读取配置：{detail}"),
                "请在外部修复 YAML 后重新读取；OMP Switch 不会覆盖错误文件。",
            ));
        }
        TargetConfigurationStatus::Writable
        | TargetConfigurationStatus::ReadOnly
        | TargetConfigurationStatus::CreationRequired => {}
    }

    let models_document = match target.models.resolved_path.as_deref() {
        Some(path) => Some(read_document(path, "models.yml")?),
        None => None,
    };
    let config_document = match target.config.resolved_path.as_deref() {
        Some(path) => Some(read_document(path, "config.yml")?),
        None => None,
    };

    let models_tree = models_document.as_ref().map(|document| &document.tree);
    let config_tree = config_document.as_ref().map(|document| &document.tree);
    let providers = models_tree
        .map(|tree| project_providers(tree))
        .unwrap_or_default();
    let models = providers
        .iter()
        .flat_map(|provider| provider.models.iter().cloned())
        .collect::<Vec<_>>();
    let roles = config_tree
        .map(|tree| project_roles(tree))
        .unwrap_or_default();
    let provider_count = providers.len();
    let model_count = models.len();
    let role_count = roles.len();
    let editable_provider_count = providers
        .iter()
        .filter(|provider| provider.editable)
        .count();

    let state = if target.status == TargetConfigurationStatus::CreationRequired {
        OverviewState::Empty
    } else if target.status != TargetConfigurationStatus::Writable {
        OverviewState::ReadOnly
    } else if providers.is_empty() {
        OverviewState::Empty
    } else if editable_provider_count == 0 {
        OverviewState::ReadOnly
    } else {
        OverviewState::Normal
    };
    let (empty_reason, next_action) = match state {
        OverviewState::Empty if target.status == TargetConfigurationStatus::CreationRequired => (
            Some("还没有可读取的规范配置文件。".to_owned()),
            Some("完成首次设置并创建 models.yml 与 config.yml。".to_owned()),
        ),
        OverviewState::Empty => (
            Some("还没有可管理的自定义 Provider。".to_owned()),
            Some("创建一个 Provider，并同时配置它的第一个模型。".to_owned()),
        ),
        _ => (None, None),
    };
    let read_only_reason = match state {
        OverviewState::ReadOnly => Some(read_only_reason(target, editable_provider_count == 0)),
        _ => None,
    };

    let dto = OverviewDto {
        state,
        omp: OmpOverviewDto {
            status: "connected",
            executable_path: executable_path.to_owned(),
            version: version.to_owned(),
        },
        target_configuration: target.clone(),
        files: OverviewFilesDto {
            models: file_dto(&target.models, models_document.as_ref()),
            config: file_dto(&target.config, config_document.as_ref()),
        },
        counts: OverviewCountsDto {
            provider_count,
            model_count,
            role_count,
        },
        providers,
        models,
        roles,
        empty_reason,
        next_action,
        read_only_reason,
    };
    let snapshot = models_document
        .zip(config_document)
        .map(|(models, config)| ConfigurationSnapshot { models, config });
    Ok(OverviewReadResult { dto, snapshot })
}

fn read_only_reason(target: &TargetConfigurationDiscovery, no_editable_provider: bool) -> String {
    if no_editable_provider && target.status == TargetConfigurationStatus::Writable {
        return "当前配置包含只读的 OMP 覆盖或高级 Provider。".to_owned();
    }
    match target.status {
        TargetConfigurationStatus::ReadOnly => {
            "当前配置只能查看；OMP Switch 不会修改 .yaml 或不可写文件。".to_owned()
        }
        _ => "当前 Target configuration 不允许安全写入。".to_owned(),
    }
}

fn file_dto(
    discovery: &crate::target_configuration::ConfigurationFileDiscovery,
    document: Option<&ParsedConfiguration>,
) -> OverviewFileDto {
    OverviewFileDto {
        canonical_path: discovery.canonical_path.clone(),
        resolved_path: discovery.resolved_path.clone(),
        status: discovery.status.clone(),
        content_hash: document.map(|document| document.raw_hash.clone()),
    }
}

fn read_document(path: &str, label: &str) -> Result<ParsedConfiguration, AppError> {
    let bytes = fs::read(path).map_err(|_| {
        AppError::new(
            "overview-read-failed",
            format!("无法读取 {label}。"),
            "请检查配置文件路径和权限后重新读取。",
        )
    })?;
    let raw_hash = content_hash(&bytes);
    let tree = serde_yaml::from_slice::<Value>(&bytes).map_err(|error| {
        AppError::new(
            "overview-parse-error",
            format!(
                "无法读取 {label}：{}",
                redact_diagnostic(&error.to_string())
            ),
            "请在外部修复 YAML 后重新读取；OMP Switch 不会覆盖错误文件。",
        )
    })?;
    Ok(ParsedConfiguration { raw_hash, tree })
}

fn content_hash(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn project_providers(tree: &Value) -> Vec<ProviderSummaryDto> {
    let Some(providers) = mapping_get_opt(mapping(tree), "providers") else {
        return Vec::new();
    };
    named_entries(providers)
        .into_iter()
        .map(|(provider_id, provider_value)| project_provider(provider_id, provider_value))
        .collect()
}

fn project_provider(provider_id: String, value: &Value) -> ProviderSummaryDto {
    let Some(provider) = mapping(value) else {
        return ProviderSummaryDto {
            id: provider_id,
            name: None,
            base_url: None,
            default_api: None,
            auth_mode: "unsupported".to_owned(),
            has_api_key: false,
            model_count: 0,
            editable: false,
            read_only_reason: Some("Provider 配置不是可识别的对象。".to_owned()),
            models: Vec::new(),
        };
    };
    let known = [
        "id",
        "name",
        "baseUrl",
        "base_url",
        "api",
        "defaultApi",
        "default_api",
        "apiKey",
        "api_key",
        "authMode",
        "auth_mode",
        "models",
    ];
    let mut read_only_reason = unknown_reason(provider, &known);
    let default_api_raw = mapping_get_any(provider, &["api", "defaultApi", "default_api"]);
    let default_api = supported_api(default_api_raw);
    if default_api_raw.is_some() && default_api.is_none() {
        read_only_reason.get_or_insert_with(|| "Provider 使用了不支持的协议。".to_owned());
    }
    let (auth_mode, has_api_key, unsupported_credential) = credential_projection(provider);
    if unsupported_credential {
        read_only_reason.get_or_insert_with(|| "Provider 使用了不支持的凭据配置。".to_owned());
    }
    let models = mapping_get(provider, "models")
        .map(|models| {
            named_entries(models)
                .into_iter()
                .map(|(model_id, model_value)| {
                    project_model(
                        provider_id.clone(),
                        model_id,
                        model_value,
                        default_api.as_deref(),
                        read_only_reason.is_some(),
                    )
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if mapping_get(provider, "models").is_some() && models.is_empty() {
        read_only_reason.get_or_insert_with(|| "Provider 没有可识别的模型定义。".to_owned());
    }
    let editable = read_only_reason.is_none();
    ProviderSummaryDto {
        id: provider_id,
        name: mapping_get_any(provider, &["name"])
            .and_then(scalar_string)
            .filter(|value| !value.is_empty()),
        base_url: mapping_get_any(provider, &["baseUrl", "base_url"])
            .and_then(scalar_string)
            .map(|value| safe_base_url(&value)),
        default_api,
        auth_mode,
        has_api_key,
        model_count: models.len(),
        editable,
        read_only_reason,
        models,
    }
}

fn project_model(
    provider_id: String,
    model_id: String,
    value: &Value,
    provider_api: Option<&str>,
    provider_read_only: bool,
) -> ModelSummaryDto {
    let Some(model) = mapping(value) else {
        return ModelSummaryDto {
            provider_id,
            id: model_id,
            name: None,
            effective_api: None,
            api_source: None,
            input: Vec::new(),
            reasoning: None,
            context_window: None,
            max_tokens: None,
            complete: false,
            editable: false,
            read_only_reason: Some("Model definition 不是可识别的对象。".to_owned()),
        };
    };
    let known = [
        "id",
        "name",
        "api",
        "input",
        "reasoning",
        "contextWindow",
        "context_window",
        "maxTokens",
        "max_tokens",
    ];
    let mut read_only_reason = unknown_reason(model, &known);
    let model_api_raw = mapping_get(model, "api");
    let model_api = supported_api(model_api_raw);
    if model_api_raw.is_some() && model_api.is_none() {
        read_only_reason.get_or_insert_with(|| "Model definition 使用了不支持的协议。".to_owned());
    }
    let model_api_is_set = model_api.is_some();
    let (effective_api, api_source) = match model_api.or_else(|| provider_api.map(str::to_owned)) {
        Some(api) if model_api_is_set => (Some(api), Some("model".to_owned())),
        Some(api) => (Some(api), Some("provider".to_owned())),
        None => (None, None),
    };
    let input = mapping_get(model, "input")
        .map(string_list)
        .unwrap_or_default();
    let reasoning = mapping_get(model, "reasoning").and_then(scalar_bool);
    let context_window =
        mapping_get_any(model, &["contextWindow", "context_window"]).and_then(scalar_u64);
    let max_tokens = mapping_get_any(model, &["maxTokens", "max_tokens"]).and_then(scalar_u64);
    let name = mapping_get(model, "name")
        .and_then(scalar_string)
        .filter(|value| !value.trim().is_empty());
    let complete = name.is_some()
        && !input.is_empty()
        && context_window.is_some_and(|value| value > 0)
        && max_tokens.is_some_and(|value| value > 0)
        && context_window
            .zip(max_tokens)
            .is_some_and(|(context, max)| max <= context)
        && effective_api.is_some();
    if !complete {
        read_only_reason.get_or_insert_with(|| "Model definition 配置不完整。".to_owned());
    }
    if provider_read_only {
        read_only_reason.get_or_insert_with(|| "所属 Provider 只读。".to_owned());
    }
    ModelSummaryDto {
        provider_id,
        id: model_id,
        name,
        effective_api,
        api_source,
        input,
        reasoning,
        context_window,
        max_tokens,
        complete,
        editable: read_only_reason.is_none(),
        read_only_reason,
    }
}

fn project_roles(tree: &Value) -> Vec<RoleSummaryDto> {
    let Some(roles) = mapping_get_opt(mapping(tree), "modelRoles") else {
        return Vec::new();
    };
    named_entries(roles)
        .into_iter()
        .filter_map(|(id, value)| match value {
            Value::String(selector) if !selector.trim().is_empty() => Some(RoleSummaryDto {
                id,
                status: "configured".to_owned(),
                selector: Some(safe_projection_text(selector)),
            }),
            Value::Sequence(_) | Value::Mapping(_) => Some(RoleSummaryDto {
                id,
                status: "advanced".to_owned(),
                selector: None,
            }),
            _ => None,
        })
        .collect()
}

fn credential_projection(provider: &Mapping) -> (String, bool, bool) {
    let value = mapping_get_any(provider, &["apiKey", "api_key"]);
    let has_api_key = value.is_some_and(|value| match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        _ => true,
    });
    let unsupported_credential = value.is_some_and(|value| match value {
        Value::String(value) => value.starts_with('!'),
        Value::Null => false,
        _ => true,
    });
    let explicit_mode = mapping_get_any(provider, &["authMode", "auth_mode"])
        .and_then(scalar_string)
        .unwrap_or_default();
    let auth_mode = if unsupported_credential {
        "unsupported"
    } else if has_api_key || explicit_mode.eq_ignore_ascii_case("api-key") {
        "api-key"
    } else {
        "none"
    };
    (auth_mode.to_owned(), has_api_key, unsupported_credential)
}

fn supported_api(value: Option<&Value>) -> Option<String> {
    let value = value.and_then(scalar_string)?;
    match value.as_str() {
        "openai-completions"
        | "openai-responses"
        | "anthropic-messages"
        | "google-generative-ai" => Some(value),
        _ => None,
    }
}

fn unknown_reason(map: &Mapping, known: &[&str]) -> Option<String> {
    let known = known.iter().copied().collect::<HashSet<_>>();
    if map.keys().any(|key| match key {
        Value::String(key) => !known.contains(key.as_str()),
        _ => true,
    }) {
        Some("包含 OMP Switch 不支持的高级配置。".to_owned())
    } else {
        None
    }
}

fn named_entries(value: &Value) -> Vec<(String, &Value)> {
    match value {
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(key, value)| scalar_string(key).map(|key| (key, value)))
            .collect(),
        Value::Sequence(values) => values
            .iter()
            .enumerate()
            .filter_map(|(index, value)| {
                let id = mapping(value)
                    .and_then(|map| mapping_get_any(map, &["id", "name"]))
                    .and_then(scalar_string)
                    .filter(|id| !id.trim().is_empty())
                    .unwrap_or_else(|| format!("entry-{}", index + 1));
                Some((id, value))
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn mapping(value: &Value) -> Option<&Mapping> {
    match value {
        Value::Mapping(map) => Some(map),
        _ => None,
    }
}

fn mapping_get<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    mapping_get_in(map, key)
}

fn mapping_get_opt<'a>(map: Option<&'a Mapping>, key: &str) -> Option<&'a Value> {
    map.and_then(|map| mapping_get_in(map, key))
}

fn mapping_get_in<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(&Value::String(key.to_owned()))
}
fn mapping_get_any<'a>(map: &'a Mapping, keys: &[&str]) -> Option<&'a Value> {
    keys.iter().find_map(|key| mapping_get_in(map, key))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
fn safe_base_url(value: &str) -> String {
    redact_projection(value)
}

fn safe_projection_text(value: &str) -> String {
    redact_diagnostic(value)
}

fn scalar_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        Value::String(value) => match value.to_ascii_lowercase().as_str() {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
        _ => None,
    }
}

fn scalar_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(value) => value.to_string().parse().ok(),
        Value::String(value) => value.parse().ok(),
        _ => None,
    }
}

fn string_list(value: &Value) -> Vec<String> {
    match value {
        Value::Sequence(values) => values.iter().filter_map(scalar_string).collect(),
        Value::String(value) if !value.trim().is_empty() => vec![value.clone()],
        _ => Vec::new(),
    }
}
