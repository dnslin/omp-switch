use std::{collections::HashSet, fs};

use serde::{Serialize, Serializer};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::{
    bundled_catalog::{self, BundledCatalog},
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
    pub classification: ProviderClassification,
    pub editable: bool,
    pub read_only_reason: Option<String>,
    pub models: Vec<ModelSummaryDto>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ProviderClassification {
    Custom,
    BuiltInOverride,
    Advanced,
    Unsupported,
    Unavailable,
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

    let catalog = bundled_catalog::for_version(version)?;
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
    let (providers, providers_structure_valid) = models_tree
        .map(|tree| project_providers(tree, catalog))
        .unwrap_or((Vec::new(), false));
    let models = providers
        .iter()
        .flat_map(|provider| provider.models.iter().cloned())
        .collect::<Vec<_>>();
    let (roles, roles_structure_valid) = config_tree
        .map(|tree| project_roles(tree, &providers, catalog))
        .unwrap_or((Vec::new(), false));
    let structure_invalid = !providers_structure_valid || !roles_structure_valid;
    let provider_count = providers
        .iter()
        .filter(|provider| provider.classification == ProviderClassification::Custom)
        .count();
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
    } else if structure_invalid {
        OverviewState::ReadOnly
    } else if catalog.is_none() {
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
        OverviewState::ReadOnly => Some(read_only_reason(
            target,
            &providers,
            catalog.is_none(),
            structure_invalid,
        )),
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

fn read_only_reason(
    target: &TargetConfigurationDiscovery,
    providers: &[ProviderSummaryDto],
    catalog_missing: bool,
    structure_invalid: bool,
) -> String {
    let no_editable_provider = providers.iter().all(|provider| !provider.editable);
    if structure_invalid && target.status == TargetConfigurationStatus::Writable {
        return "当前配置业务结构无法识别，只能查看；OMP Switch 不会修改未知结构。".to_owned();
    }
    if catalog_missing
        && no_editable_provider
        && target.status == TargetConfigurationStatus::Writable
    {
        return "当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。"
            .to_owned();
    }
    if target.status == TargetConfigurationStatus::Writable && no_editable_provider {
        let mut classifications = Vec::new();
        if providers
            .iter()
            .any(|provider| provider.classification == ProviderClassification::BuiltInOverride)
        {
            classifications.push("OMP 内置 Provider/Model 覆盖");
        }
        if providers
            .iter()
            .any(|provider| provider.classification == ProviderClassification::Advanced)
        {
            classifications.push("高级 Provider");
        }
        if providers
            .iter()
            .any(|provider| provider.classification == ProviderClassification::Unsupported)
        {
            classifications.push("不支持的 Provider/Model 结构");
        }
        if !classifications.is_empty() {
            return format!(
                "当前配置包含以下只读 Provider 分类：{}。",
                classifications.join("、")
            );
        }
        return "当前配置包含无法编辑的 Provider，只能查看。".to_owned();
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
    let bytes = fs::read(path).map_err(|error| {
        AppError::new(
            "overview-read-failed",
            format!(
                "无法读取 {label}。诊断代码：{}。",
                crate::error::io_error_cause(error.kind())
            ),
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

fn project_providers(
    tree: &Value,
    catalog: Option<&BundledCatalog>,
) -> (Vec<ProviderSummaryDto>, bool) {
    let Some(root) = mapping(tree) else {
        return (Vec::new(), false);
    };
    let Some(providers) = mapping_get(root, "providers") else {
        return (Vec::new(), false);
    };
    let Some(providers_map) = mapping(providers) else {
        return (Vec::new(), false);
    };
    let structure_valid = providers_map
        .keys()
        .all(|key| matches!(key, Value::String(_)));
    let providers = named_entries(providers)
        .into_iter()
        .map(|(provider_id, provider_value)| project_provider(provider_id, provider_value, catalog))
        .collect();
    (providers, structure_valid)
}

fn project_provider(
    provider_id: String,
    value: &Value,
    catalog: Option<&BundledCatalog>,
) -> ProviderSummaryDto {
    let Some(provider) = mapping(value) else {
        return ProviderSummaryDto {
            id: provider_id,
            name: None,
            base_url: None,
            default_api: None,
            auth_mode: "unsupported".to_owned(),
            has_api_key: false,
            model_count: 0,
            classification: ProviderClassification::Unsupported,
            editable: false,
            read_only_reason: Some("Provider 配置不是可识别的对象。".to_owned()),
            models: Vec::new(),
        };
    };
    let known = ["name", "baseUrl", "api", "apiKey", "models"];
    let mut field_reason = unknown_reason(provider, &known);
    let base_url_raw = mapping_get(provider, "baseUrl").and_then(scalar_string);
    let base_url_valid = base_url_raw.as_deref().is_some_and(valid_http_url);
    if !base_url_valid {
        field_reason.get_or_insert_with(|| "Provider 必须包含有效的 HTTP(S) Base URL。".to_owned());
    }
    let default_api_raw = mapping_get(provider, "api");
    let default_api = supported_api(default_api_raw);
    if default_api_raw.is_some() && default_api.is_none() {
        field_reason.get_or_insert_with(|| "Provider 使用了不支持的协议。".to_owned());
    }
    let (auth_mode, has_api_key, unsupported_credential) = credential_projection(provider);
    if unsupported_credential {
        field_reason.get_or_insert_with(|| "Provider 使用了不支持的凭据配置。".to_owned());
    }
    let models_value = mapping_get(provider, "models");
    let provider_fields_read_only = catalog.is_none()
        || catalog.is_some_and(|catalog| catalog.contains_provider(&provider_id))
        || field_reason.is_some();
    let (entries, models_structure_valid) = models_value
        .map(model_entries)
        .unwrap_or((Vec::new(), false));
    let mut models = entries
        .into_iter()
        .map(|(model_id, model_value)| {
            project_model(
                provider_id.clone(),
                model_id,
                model_value,
                default_api.as_deref(),
                provider_fields_read_only,
            )
        })
        .collect::<Vec<_>>();
    if !models_structure_valid {
        field_reason.get_or_insert_with(|| "Model definition 列表结构无法识别。".to_owned());
    }
    if models_value.is_none() || models.is_empty() {
        field_reason.get_or_insert_with(|| "Provider 没有非空模型定义。".to_owned());
    }

    let built_in_override = catalog.is_some_and(|catalog| {
        catalog.contains_provider(&provider_id)
            || models
                .iter()
                .any(|model| catalog.contains_model(&provider_id, &model.id))
    });
    let classification = if catalog.is_none() {
        ProviderClassification::Unavailable
    } else if built_in_override {
        ProviderClassification::BuiltInOverride
    } else if models_value.is_none() || models.is_empty() || !base_url_valid {
        ProviderClassification::Unsupported
    } else if field_reason.is_some() {
        ProviderClassification::Advanced
    } else {
        ProviderClassification::Custom
    };
    let editable = classification == ProviderClassification::Custom;
    let read_only_reason = match classification {
        ProviderClassification::Custom => None,
        ProviderClassification::BuiltInOverride => {
            Some("Provider 或 Model ID 覆盖 OMP bundled catalog，只能查看。".to_owned())
        }
        ProviderClassification::Unavailable => Some(
            "当前 OMP 版本没有匹配的 bundled Provider 清单，Provider 与模型管理暂时只读。"
                .to_owned(),
        ),
        ProviderClassification::Unsupported => field_reason
            .or_else(|| Some("Provider 配置不符合可管理的 Custom Provider 结构。".to_owned())),
        ProviderClassification::Advanced => {
            field_reason.or_else(|| Some("Provider 包含 OMP Switch 不支持的高级配置。".to_owned()))
        }
    };
    if !editable {
        for model in &mut models {
            model.editable = false;
            if model.read_only_reason.is_none() {
                model.read_only_reason = read_only_reason.clone();
            }
        }
    }
    ProviderSummaryDto {
        id: provider_id,
        name: mapping_get(provider, "name")
            .and_then(scalar_string)
            .filter(|value| !value.is_empty()),
        base_url: base_url_raw
            .as_deref()
            .filter(|value| !value.trim().is_empty())
            .map(safe_base_url),
        default_api,
        auth_mode,
        has_api_key,
        model_count: models.len(),
        classification,
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
        "maxTokens",
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
    let input = match mapping_get(model, "input") {
        Some(value) => string_list(value).unwrap_or_else(|| {
            read_only_reason
                .get_or_insert_with(|| "Model definition 的 input 字段格式不受支持。".to_owned());
            Vec::new()
        }),
        None => Vec::new(),
    };
    if input
        .iter()
        .any(|value| !matches!(value.as_str(), "text" | "image"))
    {
        read_only_reason
            .get_or_insert_with(|| "Model definition 使用了不支持的输入能力。".to_owned());
    }
    let reasoning = mapping_get(model, "reasoning").and_then(scalar_bool);
    let context_window = mapping_get(model, "contextWindow").and_then(scalar_u64);
    let max_tokens = mapping_get(model, "maxTokens").and_then(scalar_u64);
    let name = mapping_get(model, "name")
        .and_then(scalar_string)
        .filter(|value| !value.trim().is_empty());
    let complete = name.is_some()
        && !input.is_empty()
        && input
            .iter()
            .all(|value| matches!(value.as_str(), "text" | "image"))
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

#[derive(Clone, Copy, PartialEq, Eq)]
enum RoleModelStatus {
    Configured,
    ProviderMissing,
    ModelMissing,
    Incomplete,
}

impl RoleModelStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::ProviderMissing => "provider-missing",
            Self::ModelMissing => "model-missing",
            Self::Incomplete => "incomplete",
        }
    }
}

fn project_roles(
    tree: &Value,
    providers: &[ProviderSummaryDto],
    catalog: Option<&BundledCatalog>,
) -> (Vec<RoleSummaryDto>, bool) {
    let Some(root) = mapping(tree) else {
        return (Vec::new(), false);
    };
    let Some(roles) = mapping_get(root, "modelRoles") else {
        return (Vec::new(), false);
    };
    let Some(roles_map) = mapping(roles) else {
        return (Vec::new(), false);
    };
    let structure_valid = roles_map.keys().all(|key| matches!(key, Value::String(_)));
    let roles = named_entries(roles)
        .into_iter()
        .map(|(id, value)| match value {
            Value::Null => RoleSummaryDto {
                id,
                status: "unconfigured".to_owned(),
                selector: None,
            },
            Value::String(selector) => project_role(id, selector, providers, catalog),
            _ => RoleSummaryDto {
                id,
                status: "advanced".to_owned(),
                selector: None,
            },
        })
        .collect();
    (roles, structure_valid)
}

fn project_role(
    id: String,
    selector: &str,
    providers: &[ProviderSummaryDto],
    catalog: Option<&BundledCatalog>,
) -> RoleSummaryDto {
    let selector = selector.trim();
    let Some((provider_id, model_and_thinking)) = parse_role_selector(selector) else {
        return RoleSummaryDto {
            id,
            status: if selector.is_empty() {
                "unconfigured"
            } else {
                "advanced"
            }
            .to_owned(),
            selector: None,
        };
    };
    let full_status = resolve_role_model(provider_id, model_and_thinking, providers, catalog);
    match full_status {
        RoleModelStatus::Configured | RoleModelStatus::Incomplete => {
            role_summary(id, full_status, selector)
        }
        RoleModelStatus::ProviderMissing | RoleModelStatus::ModelMissing => {
            let Some((base_model, thinking)) = model_and_thinking.rsplit_once(':') else {
                return role_summary(id, full_status, selector);
            };
            if is_supported_thinking(thinking) {
                return role_summary(
                    id,
                    resolve_role_model(provider_id, base_model, providers, catalog),
                    selector,
                );
            }
            if matches!(
                resolve_role_model(provider_id, base_model, providers, catalog),
                RoleModelStatus::Configured | RoleModelStatus::Incomplete
            ) {
                return RoleSummaryDto {
                    id,
                    status: "advanced".to_owned(),
                    selector: None,
                };
            }
            role_summary(id, full_status, selector)
        }
    }
}

fn role_summary(id: String, status: RoleModelStatus, selector: &str) -> RoleSummaryDto {
    RoleSummaryDto {
        id,
        status: status.as_str().to_owned(),
        selector: Some(safe_projection_text(selector)),
    }
}

fn parse_role_selector(selector: &str) -> Option<(&str, &str)> {
    if selector.is_empty()
        || selector
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        || selector.contains(',')
        || selector.starts_with('@')
    {
        return None;
    }
    let mut segments = selector.splitn(2, '/');
    let provider = segments.next()?;
    let model = segments.next()?;
    if provider.is_empty() || provider.contains(':') || model.is_empty() {
        return None;
    }
    Some((provider, model))
}

fn resolve_role_model(
    provider_id: &str,
    model_id: &str,
    providers: &[ProviderSummaryDto],
    catalog: Option<&BundledCatalog>,
) -> RoleModelStatus {
    let provider = providers
        .iter()
        .find(|provider| provider.id.eq_ignore_ascii_case(provider_id));
    match provider {
        Some(provider) => {
            if let Some(model) = provider
                .models
                .iter()
                .find(|model| model.id.eq_ignore_ascii_case(model_id))
            {
                if model.complete {
                    RoleModelStatus::Configured
                } else {
                    RoleModelStatus::Incomplete
                }
            } else if catalog.is_some_and(|catalog| catalog.contains_model(provider_id, model_id)) {
                RoleModelStatus::Configured
            } else {
                RoleModelStatus::ModelMissing
            }
        }
        None => {
            if catalog.is_some_and(|catalog| catalog.contains_provider(provider_id)) {
                if catalog.is_some_and(|catalog| catalog.contains_model(provider_id, model_id)) {
                    RoleModelStatus::Configured
                } else {
                    RoleModelStatus::ModelMissing
                }
            } else {
                RoleModelStatus::ProviderMissing
            }
        }
    }
}

fn is_supported_thinking(value: &str) -> bool {
    matches!(
        value,
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto"
    )
}

fn credential_projection(provider: &Mapping) -> (String, bool, bool) {
    let value = mapping_get(provider, "apiKey");
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
    let auth_mode = if unsupported_credential {
        "unsupported"
    } else if has_api_key {
        "api-key"
    } else {
        "none"
    };
    (auth_mode.to_owned(), has_api_key, unsupported_credential)
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

fn model_entries(value: &Value) -> (Vec<(String, &Value)>, bool) {
    let Value::Sequence(values) = value else {
        return (Vec::new(), false);
    };
    let mut structure_valid = true;
    let entries = values
        .iter()
        .filter_map(|value| {
            let Some(model) = mapping(value) else {
                structure_valid = false;
                return None;
            };
            let Some(id) = mapping_get(model, "id")
                .and_then(scalar_string)
                .filter(|id| !id.trim().is_empty())
            else {
                structure_valid = false;
                return None;
            };
            Some((id, value))
        })
        .collect();
    (entries, structure_valid)
}

fn named_entries(value: &Value) -> Vec<(String, &Value)> {
    match value {
        Value::Mapping(map) => map
            .iter()
            .filter_map(|(key, value)| scalar_string(key).map(|key| (key, value)))
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

fn mapping_get_in<'a>(map: &'a Mapping, key: &str) -> Option<&'a Value> {
    map.get(&Value::String(key.to_owned()))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}
fn safe_base_url(value: &str) -> String {
    redact_projection(value.trim())
}

fn valid_http_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value.trim()) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
}

fn safe_projection_text(value: &str) -> String {
    redact_diagnostic(value)
}

fn scalar_bool(value: &Value) -> Option<bool> {
    match value {
        Value::Bool(value) => Some(*value),
        _ => None,
    }
}

fn scalar_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(value) => value.as_u64(),
        _ => None,
    }
}

fn string_list(value: &Value) -> Option<Vec<String>> {
    match value {
        Value::Sequence(values) => values.iter().map(scalar_string).collect(),
        _ => None,
    }
}
