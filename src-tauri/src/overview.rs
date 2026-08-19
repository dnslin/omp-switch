use std::{collections::HashSet, fs, io, time::Instant};

use serde::{Serialize, Serializer};
use serde_yaml::{Mapping, Value};
use sha2::{Digest, Sha256};

use crate::{
    bundled_catalog::{self, BundledCatalog},
    error::AppError,
    provider_mutation::SupportedApi,
    redaction::{redact_diagnostic, redact_projection, url_projection_is_lossless},
    target_configuration::{
        ConfigurationFileStatus, TargetConfigurationDiscovery, TargetConfigurationStatus,
    },
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverviewAuthMode {
    ApiKey,
    None,
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverviewApiSource {
    Provider,
    Model,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverviewRoleStatus {
    Configured,
    Unconfigured,
    ProviderMissing,
    ModelMissing,
    Incomplete,
    Unsupported,
    Advanced,
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
    pub roles_editable: bool,
    pub roles_assignable: bool,
    pub roles_read_only_reason: Option<String>,
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
    pub auth_mode: OverviewAuthMode,
    pub has_api_key: bool,
    pub model_count: usize,
    pub classification: ProviderClassification,
    pub editable: bool,
    pub read_only_reason: Option<String>,
    pub role_reference_paths: Vec<String>,
    pub other_reference_paths: Vec<String>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum ModelStatus {
    Normal,
    Incomplete,
    ReadOnly,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum OverviewInput {
    Text,
    Image,
    Unsupported,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSummaryDto {
    pub provider_id: String,
    pub id: String,
    pub name: Option<String>,
    pub effective_api: Option<String>,
    pub unsupported_protocol: bool,
    pub api_source: Option<OverviewApiSource>,
    pub has_base_url_override: bool,
    pub input: Vec<OverviewInput>,
    pub reasoning: Option<bool>,
    pub context_window: Option<u64>,
    pub max_tokens: Option<u64>,
    pub complete: bool,
    pub status: ModelStatus,
    pub editable: bool,
    pub reference_count: usize,
    pub reference_paths: Vec<String>,
    pub role_reference_paths: Vec<String>,
    pub other_reference_paths: Vec<String>,
    pub read_only_reason: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSummaryDto {
    pub id: String,
    pub status: OverviewRoleStatus,
    pub selector: Option<String>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub thinking_level: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ParsedConfiguration {
    pub(crate) raw_hash: String,
    pub(crate) tree: Value,
}

#[cfg(test)]
#[derive(Clone)]
pub(crate) struct ConfigurationSnapshot {
    pub(crate) models: ParsedConfiguration,
    pub(crate) config: ParsedConfiguration,
}

pub(crate) struct OverviewReadResult {
    pub(crate) dto: OverviewDto,
    #[cfg(test)]
    pub(crate) snapshot: Option<ConfigurationSnapshot>,
}

#[derive(Clone)]
pub(crate) struct ModelTestConfiguration {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) base_url: String,
    pub(crate) protocol: SupportedApi,
    pub(crate) auth_mode: OverviewAuthMode,
    pub(crate) api_key: Option<String>,
    pub(crate) target_path: String,
    pub(crate) models_hash: String,
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
    let (mut providers, providers_structure_valid) = models_tree
        .map(|tree| project_providers(tree, catalog))
        .unwrap_or((Vec::new(), false));
    annotate_model_references(&mut providers, models_tree, config_tree);
    let models = providers
        .iter()
        .flat_map(|provider| provider.models.iter().cloned())
        .collect::<Vec<_>>();
    let (roles, roles_structure_valid) = config_tree
        .map(|tree| project_roles(tree, &providers, catalog))
        .unwrap_or((Vec::new(), false));
    let advanced_roles = roles
        .iter()
        .any(|role| role.status == OverviewRoleStatus::Advanced);
    let config_writeable = target.status == TargetConfigurationStatus::Writable
        && target.writable
        && matches!(
            target.config.status,
            ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
        );
    let roles_editable = config_writeable && roles_structure_valid && !advanced_roles;
    let roles_assignable = roles_editable
        && catalog.is_some()
        && providers.iter().any(|provider| {
            provider.editable
                && provider.models.iter().any(|model| {
                    model.editable
                        && model.status == ModelStatus::Normal
                        && model.complete
                        && is_simple_role_selector(&provider.id, &model.id, None)
                })
        });
    let roles_read_only_reason = if roles_editable {
        None
    } else if !roles_structure_valid {
        Some("config.yml 的 modelRoles 结构无法安全编辑。请在外部修复后重新读取。".to_owned())
    } else if advanced_roles {
        Some("存在当前版本不支持的高级模型角色配置。".to_owned())
    } else if !config_writeable {
        Some("当前 config.yml 不允许安全保存模型角色。".to_owned())
    } else {
        Some("当前配置不允许安全编辑模型角色。".to_owned())
    };
    let structure_invalid = !providers_structure_valid || !roles_structure_valid;
    let provider_count = providers
        .iter()
        .filter(|provider| provider.classification == ProviderClassification::Custom)
        .count();
    let model_count = models.len();
    let role_count = roles
        .iter()
        .filter(|role| role.status != OverviewRoleStatus::Unconfigured)
        .count();
    let editable_provider_count = providers
        .iter()
        .filter(|provider| provider.editable)
        .count();

    let state = if target.status == TargetConfigurationStatus::CreationRequired {
        OverviewState::Empty
    } else if target.status != TargetConfigurationStatus::Writable
        || structure_invalid
        || catalog.is_none()
    {
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
        target_configuration: sanitized_target_configuration(target),
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
        roles_editable,
        roles_assignable,
        roles_read_only_reason,
        empty_reason,
        next_action,
        read_only_reason,
    };
    #[cfg(test)]
    let snapshot = models_document
        .zip(config_document)
        .map(|(models, config)| ConfigurationSnapshot { models, config });
    Ok(OverviewReadResult {
        dto,
        #[cfg(test)]
        snapshot,
    })
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

fn sanitized_target_configuration(
    target: &TargetConfigurationDiscovery,
) -> TargetConfigurationDiscovery {
    let mut target = target.clone();
    target.recovery_notice = target.recovery_notice.as_deref().map(redact_diagnostic);
    target.warnings = target
        .warnings
        .iter()
        .map(|warning| redact_diagnostic(warning))
        .collect();
    if let Some(issue) = target.issue.as_mut() {
        issue.message = redact_diagnostic(&issue.message);
    }
    target
}

fn read_document(path: &str, label: &str) -> Result<ParsedConfiguration, AppError> {
    let mut file =
        open_configuration_file(path).map_err(|error| document_read_error(label, error))?;
    ensure_regular_file(&file, label)?;
    let mut bytes = Vec::new();
    use std::io::Read;
    file.read_to_end(&mut bytes)
        .map_err(|error| document_read_error(label, error))?;
    parse_document(bytes, label)
}

fn open_configuration_file(path: &str) -> io::Result<fs::File> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        fs::OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NONBLOCK)
            .open(path)
    }
    #[cfg(not(unix))]
    {
        fs::File::open(path)
    }
}

fn ensure_regular_file(file: &fs::File, label: &str) -> Result<(), AppError> {
    let metadata = file
        .metadata()
        .map_err(|error| document_read_error(label, error))?;
    if metadata.is_file() {
        return Ok(());
    }
    Err(document_read_error(
        label,
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "configuration path is not a file",
        ),
    ))
}

fn read_document_with_deadline(
    path: &str,
    label: &str,
    cancellation: &tokio_util::sync::CancellationToken,
    deadline: Instant,
) -> Result<ParsedConfiguration, AppError> {
    let bytes = read_bytes_with_deadline(path, label, cancellation, deadline)?;
    parse_document(bytes, label)
}

fn read_bytes_with_deadline(
    path: &str,
    label: &str,
    cancellation: &tokio_util::sync::CancellationToken,
    deadline: Instant,
) -> Result<Vec<u8>, AppError> {
    if cancellation.is_cancelled() {
        return Err(AppError::new(
            "model-test-cancelled",
            "模型测试已取消。",
            "无需继续操作。",
        ));
    }
    if Instant::now() >= deadline {
        return Err(AppError::new(
            "model-test-timeout",
            "模型测试准备超时。",
            "请检查 OMP 可执行文件和配置目录后重试。",
        ));
    }
    let mut file =
        open_configuration_file(path).map_err(|error| document_read_error(label, error))?;
    ensure_regular_file(&file, label)?;
    if cancellation.is_cancelled() {
        return Err(AppError::new(
            "model-test-cancelled",
            "模型测试已取消。",
            "无需继续操作。",
        ));
    }
    if Instant::now() >= deadline {
        return Err(AppError::new(
            "model-test-timeout",
            "模型测试准备超时。",
            "请检查 OMP 可执行文件和配置目录后重试。",
        ));
    }

    #[cfg(unix)]
    {
        use std::io::{ErrorKind, Read};

        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            if cancellation.is_cancelled() {
                return Err(AppError::new(
                    "model-test-cancelled",
                    "模型测试已取消。",
                    "无需继续操作。",
                ));
            }
            if Instant::now() >= deadline {
                return Err(AppError::new(
                    "model-test-timeout",
                    "模型测试准备超时。",
                    "请检查 OMP 可执行文件和配置目录后重试。",
                ));
            }
            match file.read(&mut chunk) {
                Ok(0) => return Ok(bytes),
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error) if error.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(error) => return Err(document_read_error(label, error)),
            }
        }
    }

    #[cfg(not(unix))]
    {
        use std::io::Read;
        let mut bytes = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            if cancellation.is_cancelled() {
                return Err(AppError::new(
                    "model-test-cancelled",
                    "模型测试已取消。",
                    "无需继续操作。",
                ));
            }
            if Instant::now() >= deadline {
                return Err(AppError::new(
                    "model-test-timeout",
                    "模型测试准备超时。",
                    "请检查 OMP 可执行文件和配置目录后重试。",
                ));
            }
            match file.read(&mut chunk) {
                Ok(0) => return Ok(bytes),
                Ok(read) => bytes.extend_from_slice(&chunk[..read]),
                Err(error) => return Err(document_read_error(label, error)),
            }
        }
    }
}

fn document_read_error(label: &str, error: io::Error) -> AppError {
    AppError::new(
        "overview-read-failed",
        format!(
            "无法读取 {label}。诊断代码：{}。",
            crate::error::io_error_cause(error.kind())
        ),
        "请检查配置文件路径和权限后重新读取。",
    )
}

fn parse_document(bytes: Vec<u8>, label: &str) -> Result<ParsedConfiguration, AppError> {
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

pub(crate) fn read_model_test_configuration(
    target: &TargetConfigurationDiscovery,
    catalog: &BundledCatalog,
    provider_id: &str,
    model_id: &str,
    cancellation: &tokio_util::sync::CancellationToken,
    deadline: Instant,
) -> Result<ModelTestConfiguration, AppError> {
    if !matches!(target.status, TargetConfigurationStatus::Writable) || !target.writable {
        return Err(model_test_not_eligible(
            "当前 Target configuration 只读，不能测试模型。",
        ));
    }
    let path = target
        .models
        .resolved_path
        .as_deref()
        .unwrap_or(&target.models.canonical_path);
    let document = read_document_with_deadline(path, "models.yml", cancellation, deadline)?;
    let (providers, provider_ids_are_safe) = project_providers(&document.tree, Some(catalog));
    let provider = providers
        .iter()
        .find(|provider| provider.id == provider_id)
        .ok_or_else(|| {
            AppError::new(
                "model-test-not-eligible",
                "找不到要测试的 Provider。",
                "请重新读取配置并选择仍然存在的普通 Provider。",
            )
        })?;
    if !provider_ids_are_safe || !provider.editable {
        return Err(model_test_not_eligible(
            provider
                .read_only_reason
                .as_deref()
                .unwrap_or("高级或只读 Provider 不能测试。"),
        ));
    }
    let model = provider
        .models
        .iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| {
            AppError::new(
                "model-test-not-eligible",
                "找不到要测试的 Model definition。",
                "请重新读取配置并选择仍然存在的已保存模型。",
            )
        })?;
    if !model.editable || model.status != ModelStatus::Normal || !model.complete {
        return Err(model_test_not_eligible(
            model
                .read_only_reason
                .as_deref()
                .unwrap_or("当前 Model definition 不完整，不能测试。"),
        ));
    }
    if !model
        .input
        .iter()
        .any(|value| matches!(value, OverviewInput::Text))
    {
        return Err(model_test_not_eligible(
            "当前固定文本探针不支持仅图片输入的 Model definition。",
        ));
    }
    let protocol = model
        .effective_api
        .as_deref()
        .and_then(SupportedApi::from_str)
        .ok_or_else(|| {
            model_test_not_eligible("当前 Model definition 没有有效的 Supported protocol。")
        })?;
    let root = mapping(&document.tree)
        .ok_or_else(|| model_test_not_eligible("models.yml 结构无法识别。"))?;
    let providers_value = mapping_get(root, "providers")
        .ok_or_else(|| model_test_not_eligible("models.yml 缺少 providers。"))?;
    let providers_map = mapping(providers_value)
        .ok_or_else(|| model_test_not_eligible("models.yml 的 providers 结构无法识别。"))?;
    let provider_value = mapping_get(providers_map, provider_id)
        .ok_or_else(|| model_test_not_eligible("Provider 配置已变化，请重新读取。"))?;
    let provider_map = mapping(provider_value)
        .ok_or_else(|| model_test_not_eligible("Provider 配置结构无法识别。"))?;
    let base_url = mapping_get(provider_map, "baseUrl")
        .and_then(scalar_string)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| model_test_not_eligible("Provider 没有有效的 Base URL。"))?;
    let api_key = mapping_get(provider_map, "apiKey").and_then(|value| match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        _ => None,
    });
    Ok(ModelTestConfiguration {
        provider_id: provider_id.to_owned(),
        model_id: model_id.to_owned(),
        base_url,
        protocol,
        auth_mode: provider.auth_mode,
        api_key,
        target_path: target.path.clone(),
        models_hash: document.raw_hash,
    })
}

fn model_test_not_eligible(reason: &str) -> AppError {
    AppError::new(
        "model-test-not-eligible",
        "当前 Model definition 不满足测试条件。",
        reason,
    )
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
    let keys_are_strings = providers_map
        .keys()
        .all(|key| matches!(key, Value::String(_)));
    let mut providers = named_entries(providers)
        .into_iter()
        .map(|(provider_id, provider_value)| project_provider(provider_id, provider_value, catalog))
        .collect::<Vec<_>>();
    let mut seen_provider_ids = HashSet::new();
    let mut colliding_provider_ids = HashSet::new();
    for provider in &providers {
        let normalized = provider.id.to_ascii_lowercase();
        if !seen_provider_ids.insert(normalized.clone()) {
            colliding_provider_ids.insert(normalized);
        }
    }
    let provider_id_collision = !colliding_provider_ids.is_empty();
    if provider_id_collision {
        for provider in &mut providers {
            if !colliding_provider_ids.contains(&provider.id.to_ascii_lowercase()) {
                continue;
            }
            provider.classification = ProviderClassification::Advanced;
            provider.editable = false;
            provider.read_only_reason =
                Some("Provider ID 在全部 providers 中必须按不区分大小写唯一。".to_owned());
            for model in &mut provider.models {
                model.status = ModelStatus::ReadOnly;
                model.editable = false;
                model.read_only_reason =
                    Some("所属 Provider ID 存在不区分大小写的冲突，只能查看。".to_owned());
            }
        }
    }
    (providers, keys_are_strings && !provider_id_collision)
}

pub(crate) fn is_editable_custom_provider(
    tree: &Value,
    provider_id: &str,
    catalog: &BundledCatalog,
) -> bool {
    let (providers, provider_ids_are_safe) = project_providers(tree, Some(catalog));
    provider_ids_are_safe
        && providers
            .iter()
            .any(|provider| provider.id == provider_id && provider.editable)
}
pub(crate) fn is_editable_model_definition(
    tree: &Value,
    provider_id: &str,
    model_id: &str,
    catalog: &BundledCatalog,
) -> bool {
    let (providers, provider_ids_are_safe) = project_providers(tree, Some(catalog));
    provider_ids_are_safe
        && providers.iter().any(|provider| {
            provider.id == provider_id
                && provider.editable
                && provider
                    .models
                    .iter()
                    .any(|model| model.id == model_id && model.editable)
        })
}

pub(crate) fn is_assignable_model_definition(
    tree: &Value,
    provider_id: &str,
    model_id: &str,
    catalog: &BundledCatalog,
) -> bool {
    if !is_simple_role_selector(provider_id, model_id, None) {
        return false;
    }
    let (providers, provider_ids_are_safe) = project_providers(tree, Some(catalog));
    provider_ids_are_safe
        && providers.iter().any(|provider| {
            provider.id.eq_ignore_ascii_case(provider_id)
                && provider.editable
                && provider.models.iter().any(|model| {
                    model.id.eq_ignore_ascii_case(model_id) && model.editable && model.complete
                })
        })
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
            auth_mode: OverviewAuthMode::Unsupported,
            has_api_key: false,
            model_count: 0,
            classification: ProviderClassification::Unsupported,
            editable: false,
            read_only_reason: Some("Provider 配置不是可识别的对象。".to_owned()),
            role_reference_paths: Vec::new(),
            other_reference_paths: Vec::new(),
            models: Vec::new(),
        };
    };
    let supported_fields = ["name", "baseUrl", "api", "apiKey", "models"];
    let mut field_reason = unsupported_field_reason(provider, &supported_fields);
    let base_url_raw = mapping_get(provider, "baseUrl").and_then(scalar_string);
    let base_url_valid = base_url_raw.as_deref().is_some_and(valid_http_url);
    if !base_url_valid {
        field_reason.get_or_insert_with(|| "Provider 必须包含有效的 HTTP(S) Base URL。".to_owned());
    }
    let base_url_safe_to_edit = base_url_raw
        .as_deref()
        .is_some_and(|value| url_projection_is_lossless(value.trim()));
    if !base_url_safe_to_edit {
        field_reason
            .get_or_insert_with(|| "Base URL 包含无法安全回写的脱敏信息，只能查看。".to_owned());
    }
    let default_api_raw = mapping_get(provider, "api");
    let default_api_is_configured =
        default_api_raw.is_some_and(|value| !matches!(value, Value::Null));
    let default_api = supported_api(default_api_raw);
    if default_api_is_configured && default_api.is_none() {
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
    let mut seen_model_ids = HashSet::new();
    let model_id_collision = entries
        .iter()
        .any(|(model_id, _)| !seen_model_ids.insert(model_id.to_ascii_lowercase()));
    let mut models = entries
        .into_iter()
        .map(|(model_id, model_value)| {
            project_model(
                provider_id.clone(),
                model_id,
                model_value,
                default_api.as_deref(),
                provider_fields_read_only,
                default_api_is_configured && default_api.is_none(),
            )
        })
        .collect::<Vec<_>>();
    if !models_structure_valid {
        field_reason.get_or_insert_with(|| "Model definition 列表结构无法识别。".to_owned());
    }
    if model_id_collision {
        field_reason
            .get_or_insert_with(|| "同一 Provider 中 Model ID 必须按不区分大小写唯一。".to_owned());
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
            model.status = ModelStatus::ReadOnly;
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
        role_reference_paths: Vec::new(),
        other_reference_paths: Vec::new(),
        models,
    }
}
fn project_model(
    provider_id: String,
    model_id: String,
    value: &Value,
    provider_api: Option<&str>,
    provider_read_only: bool,
    provider_protocol_unsupported: bool,
) -> ModelSummaryDto {
    let Some(model) = mapping(value) else {
        return ModelSummaryDto {
            provider_id,
            id: model_id,
            name: None,
            effective_api: None,
            unsupported_protocol: false,
            api_source: None,
            has_base_url_override: false,
            input: Vec::new(),
            reasoning: None,
            context_window: None,
            max_tokens: None,
            complete: false,
            status: ModelStatus::ReadOnly,
            editable: false,
            reference_count: 0,
            reference_paths: Vec::new(),
            role_reference_paths: Vec::new(),
            other_reference_paths: Vec::new(),
            read_only_reason: Some("Model definition 不是可识别的对象。".to_owned()),
        };
    };
    let supported_fields = [
        "id",
        "name",
        "api",
        "input",
        "reasoning",
        "contextWindow",
        "maxTokens",
    ];
    let mut read_only_reason = unsupported_field_reason(model, &supported_fields);
    let has_base_url_override = mapping_get(model, "baseUrl").is_some();
    let model_api_raw = mapping_get(model, "api");
    let model_api_overrides_provider =
        model_api_raw.is_some_and(|value| !matches!(value, Value::Null));
    let model_api = supported_api(model_api_raw);
    let unsupported_protocol =
        provider_protocol_unsupported || (model_api_overrides_provider && model_api.is_none());
    if model_api_overrides_provider && model_api.is_none() {
        read_only_reason.get_or_insert_with(|| "Model definition 使用了不支持的协议。".to_owned());
    }
    let (effective_api, api_source) = if model_api_overrides_provider {
        match model_api {
            Some(api) => (Some(api), Some(OverviewApiSource::Model)),
            None => (None, None),
        }
    } else {
        match provider_api {
            Some(api) => (Some(api.to_owned()), Some(OverviewApiSource::Provider)),
            None => (None, None),
        }
    };
    let input = match mapping_get(model, "input") {
        Some(value) => string_list(value)
            .map(|values| {
                values
                    .into_iter()
                    .map(|value| match value.as_str() {
                        "text" => OverviewInput::Text,
                        "image" => OverviewInput::Image,
                        _ => {
                            read_only_reason.get_or_insert_with(|| {
                                "Model definition 使用了不支持的输入能力。".to_owned()
                            });
                            OverviewInput::Unsupported
                        }
                    })
                    .collect()
            })
            .unwrap_or_else(|| {
                read_only_reason.get_or_insert_with(|| {
                    "Model definition 的 input 字段格式不受支持。".to_owned()
                });
                Vec::new()
            }),
        None => Vec::new(),
    };
    let reasoning_value = mapping_get(model, "reasoning");
    let reasoning = reasoning_value.and_then(scalar_bool);
    if reasoning_value.is_some_and(|value| !matches!(value, Value::Bool(_))) {
        read_only_reason
            .get_or_insert_with(|| "Model definition 的 reasoning 字段格式不受支持。".to_owned());
    }
    let context_window_value = mapping_get(model, "contextWindow");
    let max_tokens_value = mapping_get(model, "maxTokens");
    let name_value = mapping_get(model, "name");
    let context_window = context_window_value.and_then(scalar_u64);
    let max_tokens = max_tokens_value.and_then(scalar_u64);
    let name = name_value
        .and_then(scalar_string)
        .filter(|value| !value.trim().is_empty());
    if name_value.is_some_and(|value| !matches!(value, Value::String(_) | Value::Null)) {
        read_only_reason
            .get_or_insert_with(|| "Model definition 的 name 字段格式不受支持。".to_owned());
    }
    if context_window_value
        .is_some_and(|value| !matches!(value, Value::Null) && value.as_u64().is_none())
    {
        read_only_reason.get_or_insert_with(|| {
            "Model definition 的 contextWindow 字段格式不受支持。".to_owned()
        });
    }
    if max_tokens_value
        .is_some_and(|value| !matches!(value, Value::Null) && value.as_u64().is_none())
    {
        read_only_reason
            .get_or_insert_with(|| "Model definition 的 maxTokens 字段格式不受支持。".to_owned());
    }
    let complete = name.is_some()
        && !input.is_empty()
        && input
            .iter()
            .all(|value| matches!(value, OverviewInput::Text | OverviewInput::Image))
        && context_window.is_some_and(|value| value > 0)
        && max_tokens.is_some_and(|value| value > 0)
        && matches!((context_window, max_tokens), (Some(context), Some(max)) if max <= context)
        && effective_api.is_some();
    if provider_read_only {
        read_only_reason.get_or_insert_with(|| "所属 Provider 只读。".to_owned());
    }
    let status = if read_only_reason.is_some() {
        ModelStatus::ReadOnly
    } else if complete {
        ModelStatus::Normal
    } else {
        ModelStatus::Incomplete
    };
    ModelSummaryDto {
        provider_id,
        id: model_id,
        unsupported_protocol,
        name,
        effective_api,
        api_source,
        has_base_url_override,
        input,
        reasoning,
        context_window,
        max_tokens,
        complete,
        status,
        editable: read_only_reason.is_none(),
        reference_count: 0,
        reference_paths: Vec::new(),
        role_reference_paths: Vec::new(),
        other_reference_paths: Vec::new(),
        read_only_reason,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ReferenceScan {
    pub(crate) role_paths: Vec<String>,
    pub(crate) other_paths: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectorReferenceKind {
    SupportedRole,
    Other,
}

fn annotate_model_references(
    providers: &mut [ProviderSummaryDto],
    models_tree: Option<&Value>,
    config_tree: Option<&Value>,
) {
    for provider in providers {
        let known_model_ids: Vec<String> = provider
            .models
            .iter()
            .map(|model| model.id.clone())
            .collect();

        let provider_scan = scan_model_references(
            models_tree,
            config_tree,
            &provider.id,
            None,
            &known_model_ids,
            Some(&provider_node_path(&provider.id)),
        );

        provider.role_reference_paths = provider_scan.role_paths;
        provider.other_reference_paths = provider_scan.other_paths;
        for model in &mut provider.models {
            let skip_path =
                models_tree.and_then(|tree| model_node_path(tree, &provider.id, &model.id));
            let scan = scan_model_references(
                models_tree,
                config_tree,
                &provider.id,
                Some(&model.id),
                &known_model_ids,
                skip_path.as_deref(),
            );

            model.role_reference_paths = scan.role_paths;
            model.other_reference_paths = scan.other_paths;
            model.reference_paths = model
                .role_reference_paths
                .iter()
                .chain(model.other_reference_paths.iter())
                .cloned()
                .collect();
            model.reference_count = model.reference_paths.len();
        }
    }
}

pub(crate) fn scan_model_references(
    models_tree: Option<&Value>,
    config_tree: Option<&Value>,
    provider_id: &str,
    model_id: Option<&str>,
    known_model_ids: &[String],

    skip_models_path: Option<&str>,
) -> ReferenceScan {
    let mut scan = ReferenceScan::default();
    if let Some(tree) = models_tree {
        collect_reference_paths(
            tree,
            "",
            "models.yml",
            provider_id,
            model_id,
            known_model_ids,
            skip_models_path,
            false,
            &mut scan,
        );
    }
    if let Some(tree) = config_tree {
        collect_reference_paths(
            tree,
            "",
            "config.yml",
            provider_id,
            model_id,
            known_model_ids,
            None,
            false,
            &mut scan,
        );
    }
    scan
}

pub(crate) fn provider_node_path(provider_id: &str) -> String {
    format!(
        "providers[\"{}\"]",
        escape_reference_path_component(provider_id)
    )
}

pub(crate) fn model_node_path(tree: &Value, provider_id: &str, model_id: &str) -> Option<String> {
    let providers = mapping(mapping(tree).and_then(|root| mapping_get(root, "providers"))?)?;
    let provider = providers.get(Value::String(provider_id.to_owned()))?;
    let models = mapping(provider)
        .and_then(|provider| mapping_get(provider, "models"))
        .and_then(Value::as_sequence)?;
    for (index, model) in models.iter().enumerate() {
        if model
            .as_mapping()
            .and_then(|model| model.get(Value::String("id".to_owned())))
            .and_then(Value::as_str)
            == Some(model_id)
        {
            return Some(format!(
                "{}[\"models\"][{index}]",
                provider_node_path(provider_id)
            ));
        }
    }
    None
}

fn collect_reference_paths(
    value: &Value,
    path: &str,
    file_name: &'static str,
    provider_id: &str,
    model_id: Option<&str>,
    known_model_ids: &[String],

    skip_path: Option<&str>,
    role_value: bool,
    scan: &mut ReferenceScan,
) {
    if skip_path.is_some_and(|candidate| candidate == path) {
        return;
    }
    match value {
        Value::Mapping(map) => {
            for (key, child) in map {
                let key = reference_path_component(key);
                let child_path = if path.is_empty() {
                    key
                } else {
                    format!("{path}[\"{key}\"]")
                };
                let child_role_value = file_name == "config.yml" && path == "modelRoles";
                collect_reference_paths(
                    child,
                    &child_path,
                    file_name,
                    provider_id,
                    model_id,
                    known_model_ids,
                    skip_path,
                    child_role_value,
                    scan,
                );
            }
        }
        Value::Sequence(values) => {
            for (index, child) in values.iter().enumerate() {
                collect_reference_paths(
                    child,
                    &format!("{path}[{index}]"),
                    file_name,
                    provider_id,
                    model_id,
                    known_model_ids,
                    skip_path,
                    false,
                    scan,
                );
            }
        }
        Value::String(value) => {
            if let Some(kind) =
                selector_reference_kind(value, provider_id, model_id, known_model_ids)
            {
                let path = format!("{file_name}:{path}");
                if role_value && kind == SelectorReferenceKind::SupportedRole {
                    scan.role_paths.push(path);
                } else {
                    scan.other_paths.push(path);
                }
            }
        }
        Value::Tagged(tagged) => {
            collect_reference_paths(
                &tagged.value,
                path,
                file_name,
                provider_id,
                model_id,
                known_model_ids,
                skip_path,
                false,
                scan,
            );
        }

        _ => {}
    }
}

fn reference_path_component(value: &Value) -> String {
    let serialized = value.as_str().map(str::to_owned).unwrap_or_else(|| {
        serde_yaml::to_string(value)
            .map(|serialized| serialized.trim().to_owned())
            .unwrap_or_else(|_| format!("<unserializable YAML key: {value:?}>"))
    });
    escape_reference_path_component(&serialized)
}

fn escape_reference_path_component(value: &str) -> String {
    let value = redact_diagnostic(value);
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

fn selector_reference_kind(
    value: &str,
    provider_id: &str,
    model_id: Option<&str>,
    known_model_ids: &[String],
) -> Option<SelectorReferenceKind> {
    if value.contains(',') {
        return value
            .split(',')
            .map(str::trim)
            .filter(|candidate| !candidate.is_empty())
            .any(|candidate| {
                selector_reference_kind_simple(candidate, provider_id, model_id, known_model_ids)
                    .is_some()
            })
            .then_some(SelectorReferenceKind::Other);
    }
    selector_reference_kind_simple(value, provider_id, model_id, known_model_ids)
}

fn selector_reference_kind_simple(
    value: &str,
    provider_id: &str,
    model_id: Option<&str>,
    known_model_ids: &[String],
) -> Option<SelectorReferenceKind> {
    let Some((candidate_provider, raw_candidate_model)) = value.split_once('/') else {
        return None;
    };
    if bundled_catalog::normalize_id(candidate_provider)
        != bundled_catalog::normalize_id(provider_id)
    {
        return None;
    }
    if raw_candidate_model == "*" {
        return Some(SelectorReferenceKind::Other);
    }

    let exact_model = known_model_ids
        .iter()
        .find(|known_model_id| (*known_model_id).eq_ignore_ascii_case(raw_candidate_model))
        .map(String::as_str);
    if let Some(exact_model) = exact_model {
        return match model_id {
            None => Some(SelectorReferenceKind::SupportedRole),
            Some(target) if exact_model.eq_ignore_ascii_case(target) => {
                Some(SelectorReferenceKind::SupportedRole)
            }
            Some(_) => None,
        };
    }
    let (candidate_model, simple_suffix) = match raw_candidate_model.rsplit_once(':') {
        Some((model, thinking)) if is_supported_thinking(thinking) => (model, true),
        Some((model, _)) => (model, false),
        None => (raw_candidate_model, true),
    };
    let matches_target = match model_id {
        None => true,
        Some(_) if candidate_model == "*" => true,
        Some(model_id) => {
            bundled_catalog::normalize_id(candidate_model)
                == bundled_catalog::normalize_id(model_id)
        }
    };
    if !matches_target {
        return None;
    }
    if simple_suffix
        && !candidate_model.is_empty()
        && candidate_model != "*"
        && !candidate_model.contains('/')
    {
        Some(SelectorReferenceKind::SupportedRole)
    } else {
        Some(SelectorReferenceKind::Other)
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
    let structure_valid = roles_map.keys().all(|key| match key {
        Value::String(id) => is_valid_role_id(id),
        _ => false,
    });
    let roles = named_entries(roles)
        .into_iter()
        .map(|(id, value)| match value {
            Value::Null => RoleSummaryDto {
                id,
                status: OverviewRoleStatus::Unconfigured,
                selector: None,
                provider_id: None,
                model_id: None,
                thinking_level: None,
            },
            Value::String(selector) => project_role(id, selector, providers, catalog),
            _ => RoleSummaryDto {
                id,
                status: OverviewRoleStatus::Advanced,
                selector: None,
                provider_id: None,
                model_id: None,
                thinking_level: None,
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
    let Some((provider_id, model_and_thinking)) = parse_role_selector(selector) else {
        return RoleSummaryDto {
            id,
            status: if selector.is_empty() {
                OverviewRoleStatus::Unconfigured
            } else {
                OverviewRoleStatus::Advanced
            },
            selector: None,
            provider_id: None,
            model_id: None,
            thinking_level: None,
        };
    };
    let (model_id, thinking_level) = model_and_thinking
        .rsplit_once(':')
        .filter(|(_, thinking)| is_supported_thinking(thinking))
        .map_or((model_and_thinking, None), |(model, thinking)| {
            (model, Some(thinking))
        });
    if thinking_level.is_some() && model_id.is_empty() {
        return RoleSummaryDto {
            id,
            status: OverviewRoleStatus::Advanced,
            selector: None,
            provider_id: None,
            model_id: None,
            thinking_level: None,
        };
    }
    let full_status = resolve_role_model(provider_id, model_id, providers, catalog);
    match full_status {
        OverviewRoleStatus::Configured
        | OverviewRoleStatus::Incomplete
        | OverviewRoleStatus::Unsupported => role_summary(id, full_status, selector, providers),
        OverviewRoleStatus::ProviderMissing | OverviewRoleStatus::ModelMissing => {
            if thinking_level.is_some() || !model_and_thinking.contains(':') {
                role_summary(id, full_status, selector, providers)
            } else {
                RoleSummaryDto {
                    id,
                    status: OverviewRoleStatus::Advanced,
                    selector: None,
                    provider_id: None,
                    model_id: None,
                    thinking_level: None,
                }
            }
        }
        OverviewRoleStatus::Unconfigured | OverviewRoleStatus::Advanced => {
            role_summary(id, full_status, selector, providers)
        }
    }
}

pub(crate) fn model_roles_have_advanced(
    config_tree: &Value,
    models_tree: &Value,
    catalog: Option<&BundledCatalog>,
) -> bool {
    let (providers, _) = project_providers(models_tree, catalog);
    let (roles, structure_valid) = project_roles(config_tree, &providers, catalog);
    !structure_valid
        || roles
            .iter()
            .any(|role| role.status == OverviewRoleStatus::Advanced)
}

fn role_parts(
    selector: &str,
    providers: &[ProviderSummaryDto],
) -> (Option<String>, Option<String>, Option<String>) {
    let Some((provider_id, model_and_thinking)) = parse_role_selector(selector) else {
        return (None, None, None);
    };
    let (model_id, thinking_level) = model_and_thinking
        .rsplit_once(':')
        .filter(|(_, thinking)| is_supported_thinking(thinking))
        .map_or((model_and_thinking, None), |(model, thinking)| {
            (model, Some(thinking))
        });
    let thinking_level = thinking_level.map(str::to_owned);
    let fallback = || {
        (
            Some(provider_id.to_owned()),
            Some(model_id.to_owned()),
            thinking_level.clone(),
        )
    };
    if !safe_role_component(provider_id) || !safe_role_component(model_id) {
        return (None, None, None);
    }
    let Some(provider) = providers
        .iter()
        .find(|candidate| candidate.id.eq_ignore_ascii_case(provider_id))
    else {
        return fallback();
    };
    let Some(model) = provider
        .models
        .iter()
        .find(|candidate| candidate.id.eq_ignore_ascii_case(model_id))
    else {
        return fallback();
    };
    (
        Some(provider.id.clone()),
        Some(model.id.clone()),
        thinking_level,
    )
}

fn safe_role_component(value: &str) -> bool {
    value
        .split('/')
        .all(|component| redact_diagnostic(component) == component)
}

fn safe_role_selector(selector: &str) -> String {
    let Some((provider_id, model_and_thinking)) = parse_role_selector(selector) else {
        return redact_diagnostic(selector);
    };
    let model_id = model_and_thinking
        .rsplit_once(':')
        .filter(|(_, thinking)| is_supported_thinking(thinking))
        .map_or(model_and_thinking, |(model, _)| model);
    if redact_diagnostic(selector) != selector
        || !safe_role_component(provider_id)
        || !safe_role_component(model_id)
    {
        "[已脱敏]".to_owned()
    } else {
        selector.to_owned()
    }
}

fn role_summary(
    id: String,
    status: OverviewRoleStatus,
    selector: &str,
    providers: &[ProviderSummaryDto],
) -> RoleSummaryDto {
    let (provider_id, model_id, thinking_level) = role_parts(selector, providers);
    RoleSummaryDto {
        id,
        status,
        selector: Some(safe_role_selector(selector)),
        provider_id,
        model_id,
        thinking_level,
    }
}

pub(crate) fn is_valid_role_id(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && !value.chars().any(|character| {
            character.is_whitespace() || character.is_control() || matches!(character, '/' | ',')
        })
}

pub(crate) fn parse_role_selector(selector: &str) -> Option<(&str, &str)> {
    if selector.is_empty()
        || selector
            .chars()
            .any(|character| character.is_whitespace() || character.is_control())
        || selector.contains(',')
        || selector.starts_with('@')
    {
        return None;
    }
    let (provider, model) = selector.split_once('/')?;
    if provider.is_empty() || model.is_empty() {
        return None;
    }
    Some((provider, model))
}

pub(crate) fn is_simple_role_selector(
    provider_id: &str,
    model_id: &str,
    thinking_level: Option<&str>,
) -> bool {
    let mut selector = format!("{provider_id}/{model_id}");
    if let Some(level) = thinking_level {
        selector.push(':');
        selector.push_str(level);
    }
    let Some((parsed_provider_id, parsed_model_with_thinking)) = parse_role_selector(&selector)
    else {
        return false;
    };
    let (parsed_model_id, parsed_thinking_level) = parsed_model_with_thinking
        .rsplit_once(':')
        .filter(|(_, thinking)| is_supported_thinking(thinking))
        .map_or((parsed_model_with_thinking, None), |(model, thinking)| {
            (model, Some(thinking))
        });
    parsed_provider_id == provider_id
        && parsed_model_id == model_id
        && parsed_thinking_level == thinking_level
}

fn resolve_role_model(
    provider_id: &str,
    model_id: &str,
    providers: &[ProviderSummaryDto],
    catalog: Option<&BundledCatalog>,
) -> OverviewRoleStatus {
    let mut matching_providers = providers
        .iter()
        .filter(|provider| provider.id.eq_ignore_ascii_case(provider_id));
    let Some(provider) = matching_providers.next() else {
        return if catalog.is_some_and(|catalog| catalog.contains_provider(provider_id)) {
            if catalog.is_some_and(|catalog| catalog.contains_model(provider_id, model_id)) {
                OverviewRoleStatus::Configured
            } else {
                OverviewRoleStatus::ModelMissing
            }
        } else {
            OverviewRoleStatus::ProviderMissing
        };
    };
    if matching_providers.next().is_some() {
        return OverviewRoleStatus::Advanced;
    }

    let mut matching_models = provider
        .models
        .iter()
        .filter(|model| model.id.eq_ignore_ascii_case(model_id));
    let Some(model) = matching_models.next() else {
        return if catalog.is_some_and(|catalog| catalog.contains_model(provider_id, model_id)) {
            OverviewRoleStatus::Configured
        } else {
            OverviewRoleStatus::ModelMissing
        };
    };
    if matching_models.next().is_some() {
        OverviewRoleStatus::Advanced
    } else if model.unsupported_protocol {
        OverviewRoleStatus::Unsupported
    } else if model.complete {
        OverviewRoleStatus::Configured
    } else {
        OverviewRoleStatus::Incomplete
    }
}

fn is_supported_thinking(value: &str) -> bool {
    matches!(
        value,
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto"
    )
}

fn credential_projection(provider: &Mapping) -> (OverviewAuthMode, bool, bool) {
    let value = mapping_get(provider, "apiKey");
    let has_api_key = value.is_some_and(|value| match value {
        Value::Null => false,
        Value::String(value) => !value.is_empty(),
        _ => true,
    });
    let unsupported_credential = value.is_some_and(|value| {
        is_command_credential(value) || !matches!(value, Value::Null | Value::String(_))
    });
    let api_key_mode = matches!(value, Some(Value::String(_)));
    let auth_mode = if unsupported_credential {
        OverviewAuthMode::Unsupported
    } else if api_key_mode {
        OverviewAuthMode::ApiKey
    } else {
        OverviewAuthMode::None
    };
    (auth_mode, has_api_key, unsupported_credential)
}

fn is_command_credential(value: &Value) -> bool {
    match value {
        Value::Tagged(tagged) => tagged.tag == "command",
        Value::String(value) => value.starts_with('!'),
        _ => false,
    }
}
fn unsupported_field_reason(map: &Mapping, supported_fields: &[&str]) -> Option<String> {
    let supported_fields = supported_fields.iter().copied().collect::<HashSet<_>>();
    if map.keys().any(|key| match key {
        Value::String(key) => !supported_fields.contains(key.as_str()),
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
    map.get(Value::String(key.to_owned()))
}

fn scalar_string(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        _ => None,
    }
}
fn safe_base_url(value: &str) -> String {
    let value = value.trim();
    if url_projection_is_lossless(value) {
        value.to_owned()
    } else {
        redact_projection(value)
    }
}

fn valid_http_url(value: &str) -> bool {
    let Ok(url) = url::Url::parse(value.trim()) else {
        return false;
    };
    matches!(url.scheme(), "http" | "https") && url.host_str().is_some()
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
#[cfg(all(test, unix))]
mod tests {
    use std::{
        ffi::CString,
        time::{Duration, Instant},
    };

    use tempfile::tempdir;
    use tokio_util::sync::CancellationToken;

    use super::read_document_with_deadline;

    #[test]
    fn model_test_configuration_read_rejects_non_file_paths() {
        let root = tempdir().unwrap();
        let fifo = root.path().join("models.fifo");
        let fifo_path = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
        assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);

        let started = Instant::now();
        let result = read_document_with_deadline(
            fifo.to_str().unwrap(),
            "models.yml",
            &CancellationToken::new(),
            Instant::now() + Duration::from_secs(5),
        );

        let error = match result {
            Ok(_) => panic!("non-file configuration path was accepted"),
            Err(error) => error,
        };
        assert_eq!(error.code, "overview-read-failed");
        assert!(started.elapsed() < Duration::from_millis(250));
    }
}
