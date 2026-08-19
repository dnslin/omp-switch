use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};

use crate::{
    bundled_catalog::BundledCatalog,
    error::AppError,
    models_write::{self, LoadedModels, ModelsMutation, ModelsWriteFailurePoint},
    overview,
    provider_mutation::{self, SupportedApi, SupportedInput},
    redaction::redact_diagnostic,
    target_configuration::{ConfigurationFileStatus, TargetConfigurationDiscovery},
};

const THINKING_LEVELS: [&str; 8] = [
    "off", "minimal", "low", "medium", "high", "xhigh", "max", "auto",
];

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ModelDefinitionFields {
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
pub(crate) struct ModelEditFields {
    pub(crate) name: String,
    pub(crate) api: Option<SupportedApi>,
    pub(crate) reasoning: bool,
    pub(crate) input: Vec<SupportedInput>,
    pub(crate) context_window: u64,
    pub(crate) max_tokens: u64,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CreateModelInput {
    pub(crate) opened_models_hash: String,
    pub(crate) provider_id: String,
    pub(crate) model: ModelDefinitionFields,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct EditModelInput {
    pub(crate) opened_models_hash: String,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
    pub(crate) model: ModelEditFields,
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct DeleteModelInput {
    pub(crate) opened_models_hash: String,
    pub(crate) opened_config_hash: String,
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ModelMutationResult {
    pub(crate) provider_id: String,
    pub(crate) model_id: String,
}

struct ValidatedCreate {
    provider_id: String,
    model_id: String,
    model_value: Value,
}

struct ValidatedEdit {
    provider_id: String,
    model_id: String,
    fields: ModelEditFields,
}

struct ValidatedDelete {
    provider_id: String,
    model_id: String,
    config_path: PathBuf,
    expected_target: PathBuf,
    expected_config_hash: String,
}

impl ModelsMutation for ValidatedCreate {
    fn verb(&self) -> &'static str {
        "创建模型"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "serialize_created_model",
            "无法序列化新的 Model definition",
            "请检查表单后重试；原 models.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        let models = provider_models_mut(tree, &self.provider_id)?;
        models.push(self.model_value.clone());
        Ok(())
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        let models = provider_models(candidate, &self.provider_id)?;
        let created = models.last().ok_or_else(|| {
            temporary_model_error("临时 models.yml 未包含新的 Model definition。")
        })?;
        if created != &self.model_value {
            return Err(temporary_model_error(
                "临时 models.yml 未保留已验证的 Model definition。",
            ));
        }
        ensure_models_untouched_paths_equal(
            original,
            candidate,
            &self.provider_id,
            ModelChange::Create,
            None,
        )
    }
}

impl ModelsMutation for ValidatedEdit {
    fn verb(&self) -> &'static str {
        "编辑模型"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "serialize_edited_model",
            "无法序列化 Model definition 编辑结果",
            "请检查表单后重试；原 models.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        let models = provider_models_mut(tree, &self.provider_id)?;
        let model = find_model_mut(models, &self.model_id)?;
        apply_model_fields(model, &self.fields)
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        let models = provider_models(candidate, &self.provider_id)?;
        let model = find_model(models, &self.model_id)?;
        validate_model_value(model, &self.fields)?;
        ensure_models_untouched_paths_equal(
            original,
            candidate,
            &self.provider_id,
            ModelChange::Edit,
            Some(&self.model_id),
        )
    }
}

impl ModelsMutation for ValidatedDelete {
    fn verb(&self) -> &'static str {
        "删除模型"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "serialize_deleted_model",
            "无法序列化 Model definition 删除结果",
            "请重试；原 models.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        let models = provider_models_mut(tree, &self.provider_id)?;
        let index = find_model_index(models, &self.model_id)?;
        if models.len() <= 1 {
            return Err(last_model_error());
        }
        models.remove(index);
        Ok(())
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        let models = provider_models(candidate, &self.provider_id)?;
        if models.is_empty() {
            return Err(last_model_error());
        }
        if find_model(models, &self.model_id).is_ok() {
            return Err(temporary_model_error(
                "临时 models.yml 仍包含待删除的 Model definition。",
            ));
        }
        ensure_models_untouched_paths_equal(
            original,
            candidate,
            &self.provider_id,
            ModelChange::Delete,
            Some(&self.model_id),
        )
    }

    fn validate_before_commit(&self, _loaded: &LoadedModels) -> Result<(), AppError> {
        models_write::ensure_resolved_file_path(
            &self.config_path,
            &self.expected_target,
            "config.yml",
        )
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
        let bytes = fs::read(&self.config_path).map_err(|error| {
            AppError::new(
                "model-delete-unavailable",
                format!("无法在提交删除前读取 config.yml：{}", error.kind()),
                "请重新读取配置后重试；OMP Switch 不会提交可能破坏引用的删除。",
            )
        })?;
        if models_write::content_hash(&bytes) != self.expected_config_hash {
            return Err(AppError::new(
                "config-hash-conflict",
                "config.yml 在删除提交前已被外部修改。",
                "请重新读取配置；OMP Switch 不会自动合并引用变化。",
            ));
        }
        Ok(())
    }
    #[cfg(test)]
    fn mutate_external_state_for_test(&self) {
        fs::write(
            &self.config_path,
            format!(
                "modelRoles:\n  external: {}/{}\n",
                self.provider_id, self.model_id
            ),
        )
        .expect("test config mutation must succeed");
    }
}

pub(crate) fn create_model(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &CreateModelInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<ModelMutationResult, AppError> {
    let loaded = models_write::load_models_for_write(target, &input.opened_models_hash)
        .map_err(|error| remap_model_error(error, ModelOperation::Create))?;
    let validated = validate_create_input(input, &loaded.original_tree, catalog)?;
    let result = ModelMutationResult {
        provider_id: validated.provider_id.clone(),
        model_id: validated.model_id.clone(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(|error| remap_model_error(error, ModelOperation::Create))?;
    Ok(result)
}

pub(crate) fn edit_model(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &EditModelInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<ModelMutationResult, AppError> {
    let loaded = models_write::load_models_for_write(target, &input.opened_models_hash)
        .map_err(|error| remap_model_error(error, ModelOperation::Edit))?;
    let validated = validate_edit_input(input, &loaded.original_tree, catalog)?;
    let result = ModelMutationResult {
        provider_id: validated.provider_id.clone(),
        model_id: validated.model_id.clone(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(|error| remap_model_error(error, ModelOperation::Edit))?;
    Ok(result)
}

pub(crate) fn delete_model(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: &BundledCatalog,
    input: &DeleteModelInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<ModelMutationResult, AppError> {
    let loaded = models_write::load_models_for_write(target, &input.opened_models_hash)
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    let config = load_config_for_reference_check(target, &input.opened_config_hash)?;
    let validated = validate_delete_input(input, &loaded.original_tree, &config, catalog)?;
    let result = ModelMutationResult {
        provider_id: validated.provider_id.clone(),
        model_id: validated.model_id.clone(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    Ok(result)
}

struct LoadedConfig {
    tree: Value,
    config_path: PathBuf,
    expected_target: PathBuf,
    original_hash: String,
}

fn load_config_for_reference_check(
    target: &TargetConfigurationDiscovery,
    opened_config_hash: &str,
) -> Result<LoadedConfig, AppError> {
    models_write::validate_writable_target(target)
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    if !matches!(
        target.config.status,
        ConfigurationFileStatus::Normal | ConfigurationFileStatus::CanonicalWithAlternate
    ) {
        return Err(AppError::new(
            "model-delete-unavailable",
            "当前 config.yml 不允许安全检查模型引用。",
            "请重新读取配置并处理 config.yml 的当前状态。",
        ));
    }
    let expected_target =
        models_write::resolved_path(&target.resolved_path, "Target configuration")
            .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    let config_path = models_write::resolved_path(&target.config.resolved_path, "config.yml")
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    models_write::ensure_resolved_file_path(&config_path, &expected_target, "config.yml")
        .map_err(|error| remap_model_error(error, ModelOperation::Delete))?;
    let bytes = fs::read(&config_path).map_err(|error| {
        AppError::new(
            "model-delete-unavailable",
            format!("无法读取 config.yml：{}", error.kind()),
            "请修复配置文件权限后重新读取。",
        )
    })?;
    if models_write::content_hash(&bytes) != opened_config_hash {
        return Err(AppError::new(
            "config-hash-conflict",
            "config.yml 在打开删除确认后已被外部修改。",
            "请重新读取配置；OMP Switch 不会自动合并引用变化。",
        ));
    }
    let tree = serde_yaml::from_slice(&bytes).map_err(config_parse_error_from_yaml)?;
    Ok(LoadedConfig {
        tree,
        config_path,
        expected_target,
        original_hash: models_write::content_hash(&bytes),
    })
}

fn config_parse_error_from_yaml(error: serde_yaml::Error) -> AppError {
    config_parse_error(&error.to_string())
}

fn config_parse_error(diagnostic: &str) -> AppError {
    AppError::new(
        "model-delete-config-parse-error",
        format!("config.yml 无法重新解析：{}", redact_diagnostic(diagnostic)),
        "请在外部修复 YAML 后重新读取；OMP Switch 不会删除模型。",
    )
}

fn validate_create_input(
    input: &CreateModelInput,
    original_tree: &Value,
    catalog: &BundledCatalog,
) -> Result<ValidatedCreate, AppError> {
    let provider_id = validate_provider_id(&input.provider_id)?;
    ensure_editable_provider(original_tree, &provider_id, catalog, ModelOperation::Create)?;
    let providers = providers_mapping(original_tree)?;
    let provider = providers
        .get(Value::String(provider_id.clone()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| model_not_found("Provider"))?;
    let models = provider_models_from_provider(provider)?;
    let model_id = provider_mutation::normalize_model_id(&input.model.id)?;
    validate_model_id_suffix(&model_id)?;
    if models
        .iter()
        .filter_map(model_id_value)
        .any(|id| id.eq_ignore_ascii_case(&model_id))
    {
        return Err(AppError::new(
            "model-id-conflict",
            "Model ID 与同一 Provider 下的现有模型冲突。",
            "请选择一个不区分大小写也唯一的 Model ID。",
        ));
    }
    if catalog.contains_model(&provider_id, &model_id) {
        return Err(AppError::new(
            "model-id-conflict",
            "Model ID 与 OMP bundled catalog 冲突。",
            "请选择一个不区分大小写也不与 bundled catalog 冲突的 Model ID。",
        ));
    }
    validate_model_fields(&input.model, provider_api(provider))?;
    Ok(ValidatedCreate {
        provider_id,
        model_id: model_id.clone(),
        model_value: model_value(&model_id, &input.model),
    })
}

fn validate_edit_input(
    input: &EditModelInput,
    original_tree: &Value,
    catalog: &BundledCatalog,
) -> Result<ValidatedEdit, AppError> {
    let provider_id = validate_provider_id(&input.provider_id)?;
    ensure_editable_provider(original_tree, &provider_id, catalog, ModelOperation::Edit)?;
    let providers = providers_mapping(original_tree)?;
    let provider = providers
        .get(Value::String(provider_id.clone()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| model_not_found("Provider"))?;
    let models = provider_models_from_provider(provider)?;
    if find_model(models, &input.model_id).is_err() {
        if models
            .iter()
            .filter_map(model_id_value)
            .any(|id| id.eq_ignore_ascii_case(&input.model_id))
        {
            return Err(stable_id_error());
        }
        return Err(model_not_found("Model definition"));
    }
    if !overview::is_editable_model_definition(
        original_tree,
        &provider_id,
        &input.model_id,
        catalog,
    ) {
        return Err(read_only_model_error());
    }
    validate_model_fields(&input.model, provider_api(provider))?;
    Ok(ValidatedEdit {
        provider_id,
        model_id: input.model_id.clone(),
        fields: input.model.clone(),
    })
}

fn validate_delete_input(
    input: &DeleteModelInput,
    original_tree: &Value,
    config: &LoadedConfig,
    catalog: &BundledCatalog,
) -> Result<ValidatedDelete, AppError> {
    let provider_id = validate_provider_id(&input.provider_id)?;
    ensure_editable_provider(original_tree, &provider_id, catalog, ModelOperation::Delete)?;
    let providers = providers_mapping(original_tree)?;
    let provider = providers
        .get(Value::String(provider_id.clone()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| model_not_found("Provider"))?;
    let models = provider_models_from_provider(provider)?;
    if find_model(models, &input.model_id).is_err() {
        if models
            .iter()
            .filter_map(model_id_value)
            .any(|id| id.eq_ignore_ascii_case(&input.model_id))
        {
            return Err(stable_id_error());
        }
        return Err(model_not_found("Model definition"));
    }
    if !overview::is_editable_model_definition(
        original_tree,
        &provider_id,
        &input.model_id,
        catalog,
    ) {
        return Err(read_only_model_error());
    }
    let known_model_ids: Vec<String> = models
        .iter()
        .filter_map(model_id_value)
        .map(str::to_owned)
        .collect();

    let skip_path = overview::model_node_path(original_tree, &provider_id, &input.model_id);
    let references = overview::scan_model_references(
        Some(original_tree),
        Some(&config.tree),
        &provider_id,
        Some(&input.model_id),
        &known_model_ids,
        skip_path.as_deref(),
    );
    if !references.other_paths.is_empty() {
        return Err(AppError::new(
            "model-delete-unmanaged-reference",
            format!(
                "无法删除 {}/{}：非受管配置路径仍有引用：{}",
                provider_id,
                input.model_id,
                references.other_paths.join("、")
            ),
            "请先在 OMP 或外部编辑器中处理这些路径；OMP Switch 不会自动修改非受管配置。",
        ));
    }
    if !references.role_paths.is_empty() {
        return Err(AppError::new(
            "model-delete-role-reference",
            format!(
                "无法删除 {}/{}：受支持 Model role 仍有引用：{}",
                provider_id,
                input.model_id,
                references.role_paths.join("、")
            ),
            "请转入 Configuration transaction，同时更新 models.yml 和 config.yml；当前不会部分删除。",
        ));
    }
    if models.len() <= 1 {
        return Err(last_model_error());
    }
    Ok(ValidatedDelete {
        provider_id,
        model_id: input.model_id.clone(),
        config_path: config.config_path.clone(),
        expected_target: config.expected_target.clone(),
        expected_config_hash: config.original_hash.clone(),
    })
}

fn validate_provider_id(value: &str) -> Result<String, AppError> {
    if value.is_empty() || value.trim() != value {
        return Err(AppError::new(
            "provider-id-immutable",
            "已有 Provider ID 是 Stable ID，不能修改。",
            "请重新读取配置并使用当前 Provider ID。",
        ));
    }
    Ok(value.to_owned())
}

fn ensure_editable_provider(
    tree: &Value,
    provider_id: &str,
    catalog: &BundledCatalog,
    operation: ModelOperation,
) -> Result<(), AppError> {
    if overview::is_editable_custom_provider(tree, provider_id, catalog) {
        return Ok(());
    }
    Err(AppError::new(
        operation.unavailable_code(),
        "当前 Provider 不是可编辑的普通 Custom Provider。",
        "高级配置、内置覆盖和不支持的 Provider 保持只读。",
    ))
}

trait ModelFieldsView {
    fn name(&self) -> &str;
    fn api(&self) -> Option<SupportedApi>;
    fn input(&self) -> &[SupportedInput];
    fn context_window(&self) -> u64;
    fn max_tokens(&self) -> u64;
}

impl ModelFieldsView for ModelDefinitionFields {
    fn name(&self) -> &str {
        &self.name
    }
    fn api(&self) -> Option<SupportedApi> {
        self.api
    }
    fn input(&self) -> &[SupportedInput] {
        &self.input
    }
    fn context_window(&self) -> u64 {
        self.context_window
    }
    fn max_tokens(&self) -> u64 {
        self.max_tokens
    }
}

impl ModelFieldsView for ModelEditFields {
    fn name(&self) -> &str {
        &self.name
    }
    fn api(&self) -> Option<SupportedApi> {
        self.api
    }
    fn input(&self) -> &[SupportedInput] {
        &self.input
    }
    fn context_window(&self) -> u64 {
        self.context_window
    }
    fn max_tokens(&self) -> u64 {
        self.max_tokens
    }
}

fn validate_model_fields<T: ModelFieldsView>(
    fields: &T,
    provider_api: Option<&str>,
) -> Result<(), AppError> {
    if fields.name().trim().is_empty() {
        return Err(AppError::new(
            "model-name-required",
            "Model 名称不能为空。",
            "请填写 Model definition 的名称。",
        ));
    }
    if fields.input().is_empty() {
        return Err(AppError::new(
            "model-input-required",
            "Model 至少需要支持 Text 或 Image 一种输入。",
            "请选择 Text、Image 或两者。",
        ));
    }
    if fields.context_window() == 0 {
        return Err(AppError::new(
            "model-context-window-invalid",
            "Context Window 必须是正整数。",
            "请输入大于 0 的 Context Window。",
        ));
    }
    if fields.max_tokens() == 0 || fields.max_tokens() > fields.context_window() {
        return Err(AppError::new(
            "model-token-limit-invalid",
            if fields.max_tokens() == 0 {
                "Max Tokens 必须是正整数。"
            } else {
                "Max Tokens 不能大于 Context Window。"
            },
            "请填写不大于 Context Window 的正整数 Max Tokens。",
        ));
    }
    let mut inputs = HashSet::new();
    if fields
        .input()
        .iter()
        .any(|input| !inputs.insert(input.as_str()))
    {
        return Err(AppError::new(
            "model-input-invalid",
            "Model 输入能力不能重复。",
            "请保留每种输入能力最多一项。",
        ));
    }
    if fields.api().is_none() && provider_api.is_none() {
        return Err(AppError::new(
            "model-api-required",
            "Model 必须从 Provider 默认协议或模型协议覆盖获得有效协议。",
            "请选择 Provider 默认协议，或为该模型选择协议覆盖。",
        ));
    }
    Ok(())
}

fn validate_model_id_suffix(model_id: &str) -> Result<(), AppError> {
    if THINKING_LEVELS
        .iter()
        .any(|level| model_id.ends_with(&format!(":{level}")))
    {
        return Err(AppError::new(
            "model-id-invalid",
            "Model ID 不能以 Thinking Level 后缀结尾。",
            "请移除 :off、:minimal、:low、:medium、:high、:xhigh、:max 或 :auto 后缀。",
        ));
    }
    Ok(())
}

fn model_value(id: &str, fields: &ModelDefinitionFields) -> Value {
    let mut model = Mapping::new();
    model.insert(key("id"), Value::String(id.to_owned()));
    model.insert(key("name"), Value::String(fields.name.trim().to_owned()));
    if let Some(api) = fields.api {
        model.insert(key("api"), Value::String(api.as_str().to_owned()));
    }
    model.insert(key("reasoning"), Value::Bool(fields.reasoning));
    model.insert(
        key("input"),
        Value::Sequence(
            fields
                .input
                .iter()
                .map(|input| Value::String(input.as_str().to_owned()))
                .collect(),
        ),
    );
    model.insert(
        key("contextWindow"),
        Value::Number(fields.context_window.into()),
    );
    model.insert(key("maxTokens"), Value::Number(fields.max_tokens.into()));
    Value::Mapping(model)
}

fn apply_model_fields(value: &mut Value, fields: &ModelEditFields) -> Result<(), AppError> {
    let Some(model) = value.as_mapping_mut() else {
        return Err(temporary_model_error(
            "临时 models.yml 中的 Model definition 不是对象。",
        ));
    };
    model.insert(key("name"), Value::String(fields.name.trim().to_owned()));
    match fields.api {
        Some(api) => {
            model.insert(key("api"), Value::String(api.as_str().to_owned()));
        }
        None => {
            model.remove(key("api"));
        }
    }
    model.insert(key("reasoning"), Value::Bool(fields.reasoning));
    model.insert(
        key("input"),
        Value::Sequence(
            fields
                .input
                .iter()
                .map(|input| Value::String(input.as_str().to_owned()))
                .collect(),
        ),
    );
    model.insert(
        key("contextWindow"),
        Value::Number(fields.context_window.into()),
    );
    model.insert(key("maxTokens"), Value::Number(fields.max_tokens.into()));
    Ok(())
}

fn validate_model_value(value: &Value, fields: &ModelEditFields) -> Result<(), AppError> {
    let Some(model) = value.as_mapping() else {
        return Err(temporary_model_error(
            "临时 models.yml 中的 Model definition 不是对象。",
        ));
    };
    if model.get(key("name")).and_then(Value::as_str) != Some(fields.name.trim()) {
        return Err(temporary_model_error("临时 models.yml 未保留 Model 名称。"));
    }
    let expected_api = fields.api.map(SupportedApi::as_str);
    let actual_api = model.get(key("api")).and_then(Value::as_str);
    if actual_api != expected_api {
        return Err(temporary_model_error(
            "临时 models.yml 未保留 Model 协议设置。",
        ));
    }
    if model.get(key("reasoning")).and_then(Value::as_bool) != Some(fields.reasoning)
        || model.get(key("contextWindow")).and_then(Value::as_u64) != Some(fields.context_window)
        || model.get(key("maxTokens")).and_then(Value::as_u64) != Some(fields.max_tokens)
    {
        return Err(temporary_model_error(
            "临时 models.yml 未保留 Model 支持字段。",
        ));
    }
    let expected_input = fields
        .input
        .iter()
        .map(|input| Value::String(input.as_str().to_owned()))
        .collect::<Vec<_>>();
    if model.get(key("input")) != Some(&Value::Sequence(expected_input)) {
        return Err(temporary_model_error(
            "临时 models.yml 未保留 Model 输入能力。",
        ));
    }
    Ok(())
}

enum ModelChange {
    Create,
    Edit,
    Delete,
}

fn ensure_models_untouched_paths_equal(
    original: &Value,
    candidate: &Value,
    provider_id: &str,
    change: ModelChange,
    model_id: Option<&str>,
) -> Result<(), AppError> {
    let original_root = original.as_mapping().ok_or_else(models_structure_error)?;
    let candidate_root = candidate.as_mapping().ok_or_else(untouched_value_error)?;
    if original_root.len() != candidate_root.len() {
        return Err(untouched_value_error());
    }
    for (key_value, original_value) in original_root {
        if key_value.as_str() != Some("providers")
            && candidate_root.get(key_value) != Some(original_value)
        {
            return Err(untouched_value_error());
        }
    }
    let original_providers = original_root
        .get(key("providers"))
        .and_then(Value::as_mapping)
        .ok_or_else(models_structure_error)?;
    let candidate_providers = candidate_root
        .get(key("providers"))
        .and_then(Value::as_mapping)
        .ok_or_else(untouched_value_error)?;
    if original_providers.len() != candidate_providers.len() {
        return Err(untouched_value_error());
    }
    for (provider_key, original_provider) in original_providers {
        let Some(candidate_provider) = candidate_providers.get(provider_key) else {
            return Err(untouched_value_error());
        };
        if provider_key.as_str() != Some(provider_id) {
            if candidate_provider != original_provider {
                return Err(untouched_value_error());
            }
            continue;
        }
        ensure_provider_untouched_fields_equal(original_provider, candidate_provider)?;
        let original_models = provider_models(original, provider_id)?;
        let candidate_models = provider_models(candidate, provider_id)?;
        match change {
            ModelChange::Create => {
                if candidate_models.len() != original_models.len() + 1
                    || candidate_models[..original_models.len()] != original_models[..]
                {
                    return Err(untouched_value_error());
                }
            }
            ModelChange::Delete => {
                let target_id = model_id.ok_or_else(untouched_value_error)?;
                if candidate_models.len() + 1 != original_models.len() {
                    return Err(untouched_value_error());
                }
                let target_index = find_model_index(original_models, target_id)?;
                let mut expected = original_models.to_vec();
                expected.remove(target_index);
                if candidate_models != expected.as_slice() {
                    return Err(untouched_value_error());
                }
            }
            ModelChange::Edit => {
                let target_id = model_id.ok_or_else(untouched_value_error)?;
                if candidate_models.len() != original_models.len() {
                    return Err(untouched_value_error());
                }
                for (index, original_model) in original_models.iter().enumerate() {
                    let candidate_model = &candidate_models[index];
                    if model_id_value(original_model) == Some(target_id) {
                        ensure_model_untouched_fields_equal(original_model, candidate_model)?;
                    } else if candidate_model != original_model {
                        return Err(untouched_value_error());
                    }
                }
            }
        }
    }
    Ok(())
}

fn ensure_provider_untouched_fields_equal(
    original: &Value,
    candidate: &Value,
) -> Result<(), AppError> {
    let original = original.as_mapping().ok_or_else(untouched_value_error)?;
    let candidate = candidate.as_mapping().ok_or_else(untouched_value_error)?;
    for (field, value) in original {
        if field.as_str() == Some("models") {
            continue;
        }
        if candidate.get(field) != Some(value) {
            return Err(untouched_value_error());
        }
    }
    for (field, value) in candidate {
        if field.as_str() == Some("models") {
            continue;
        }
        if original.get(field) != Some(value) {
            return Err(untouched_value_error());
        }
    }
    Ok(())
}

fn ensure_model_untouched_fields_equal(
    original: &Value,
    candidate: &Value,
) -> Result<(), AppError> {
    let original = original.as_mapping().ok_or_else(untouched_value_error)?;
    let candidate = candidate.as_mapping().ok_or_else(untouched_value_error)?;
    let supported = [
        "id",
        "name",
        "api",
        "reasoning",
        "input",
        "contextWindow",
        "maxTokens",
    ];
    for (field, value) in original {
        if supported.contains(&field.as_str().unwrap_or_default()) {
            continue;
        }
        if candidate.get(field) != Some(value) {
            return Err(untouched_value_error());
        }
    }
    for (field, value) in candidate {
        if supported.contains(&field.as_str().unwrap_or_default()) {
            continue;
        }
        if original.get(field) != Some(value) {
            return Err(untouched_value_error());
        }
    }
    Ok(())
}

fn providers_mapping(tree: &Value) -> Result<&Mapping, AppError> {
    tree.as_mapping()
        .and_then(|root| root.get(key("providers")))
        .and_then(Value::as_mapping)
        .ok_or_else(models_structure_error)
}

fn provider_models_from_provider(provider: &Mapping) -> Result<&[Value], AppError> {
    provider
        .get(key("models"))
        .and_then(Value::as_sequence)
        .map(Vec::as_slice)
        .ok_or_else(|| {
            AppError::new(
                "model-management-unavailable",
                "当前 Provider 的 Model definition 列表结构不适合安全编辑。",
                "请在外部修复 models.yml 后重新读取。",
            )
        })
}

fn provider_models<'a>(tree: &'a Value, provider_id: &str) -> Result<&'a [Value], AppError> {
    providers_mapping(tree)?
        .get(Value::String(provider_id.to_owned()))
        .and_then(Value::as_mapping)
        .ok_or_else(|| model_not_found("Provider"))
        .and_then(provider_models_from_provider)
}

fn provider_models_mut<'a>(
    tree: &'a mut Value,
    provider_id: &str,
) -> Result<&'a mut Vec<Value>, AppError> {
    tree.as_mapping_mut()
        .and_then(|root| root.get_mut(key("providers")))
        .and_then(Value::as_mapping_mut)
        .and_then(|providers| providers.get_mut(Value::String(provider_id.to_owned())))
        .and_then(Value::as_mapping_mut)
        .and_then(|provider| provider.get_mut(key("models")))
        .and_then(Value::as_sequence_mut)
        .ok_or_else(|| {
            AppError::new(
                "model-management-unavailable",
                "当前 Provider 的 Model definition 列表结构不适合安全编辑。",
                "请在外部修复 models.yml 后重新读取。",
            )
        })
}

fn find_model<'a>(models: &'a [Value], model_id: &str) -> Result<&'a Value, AppError> {
    models
        .iter()
        .find(|model| model_id_value(model) == Some(model_id))
        .ok_or_else(|| model_not_found("Model definition"))
}

fn find_model_mut<'a>(models: &'a mut [Value], model_id: &str) -> Result<&'a mut Value, AppError> {
    models
        .iter_mut()
        .find(|model| model_id_value(model) == Some(model_id))
        .ok_or_else(|| temporary_model_error("临时 models.yml 未包含要编辑的 Model definition。"))
}

fn find_model_index(models: &[Value], model_id: &str) -> Result<usize, AppError> {
    models
        .iter()
        .position(|model| model_id_value(model) == Some(model_id))
        .ok_or_else(|| temporary_model_error("临时 models.yml 未包含目标 Model definition。"))
}

fn model_id_value(value: &Value) -> Option<&str> {
    value
        .as_mapping()
        .and_then(|model| model.get(key("id")))
        .and_then(Value::as_str)
}

fn provider_api(provider: &Mapping) -> Option<&str> {
    provider
        .get(key("api"))
        .and_then(Value::as_str)
        .filter(|api| {
            matches!(
                *api,
                "openai-completions"
                    | "openai-responses"
                    | "anthropic-messages"
                    | "google-generative-ai"
            )
        })
}

fn key(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn models_structure_error() -> AppError {
    AppError::new(
        "models-structure-invalid",
        "models.yml 的 providers 结构不适合安全管理 Model definition。",
        "请在外部修复 models.yml 后重新读取；OMP Switch 不会覆盖该文件。",
    )
}

fn untouched_value_error() -> AppError {
    AppError::new(
        "models-untouched-path-changed",
        "序列化会改变未触及的配置路径；OMP Switch 已停止写入。",
        "请重新读取配置并重试；原 models.yml 没有被修改。",
    )
}

fn temporary_model_error(message: &str) -> AppError {
    AppError::new(
        "models-temporary-validation-error",
        message,
        "请重试；原 models.yml 没有被修改。",
    )
}

fn model_not_found(kind: &str) -> AppError {
    AppError::new(
        "model-not-found",
        format!("要操作的 {kind} 已不存在。"),
        "请重新读取配置后选择当前存在的 Model definition。",
    )
}

fn stable_id_error() -> AppError {
    AppError::new(
        "model-id-immutable",
        "已有 Model ID 是 Stable ID，不能修改大小写或替换。",
        "请保留当前 Model ID；如需新 ID，请复制模型后处理引用。",
    )
}

fn read_only_model_error() -> AppError {
    AppError::new(
        "model-read-only",
        "当前 Model definition 包含不支持的配置，只能查看。",
        "请保留该模型原样；高级、不支持和 Provider 只读模型不能通过此处修改。",
    )
}

fn last_model_error() -> AppError {
    AppError::new(
        "model-last-definition",
        "不能删除 Provider 下的最后一个 Model definition。",
        "请先转入 Provider 删除流程，或保留至少一个模型。",
    )
}

#[derive(Clone, Copy)]
enum ModelOperation {
    Create,
    Edit,
    Delete,
}

impl ModelOperation {
    fn unavailable_code(self) -> &'static str {
        match self {
            Self::Create => "model-create-unavailable",
            Self::Edit => "model-edit-unavailable",
            Self::Delete => "model-delete-unavailable",
        }
    }

    fn error_codes(self) -> models_write::ModelsWriteErrorCodes {
        let failed = match self {
            Self::Create => "model-create-failed",
            Self::Edit => "model-edit-failed",
            Self::Delete => "model-delete-failed",
        };
        models_write::ModelsWriteErrorCodes {
            unavailable: self.unavailable_code(),
            target_changed: self.unavailable_code(),
            failed,
        }
    }
}

fn remap_model_error(error: AppError, operation: ModelOperation) -> AppError {
    models_write::remap_models_write_error(error, operation.error_codes())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delete_config_parse_diagnostics_are_redacted() {
        let error = config_parse_error("safe-context,api_key=punctuated-secret");

        assert_eq!(error.code, "model-delete-config-parse-error");
        assert!(!error.message.contains("punctuated-secret"));
        assert!(error.message.contains("[诊断信息因可能包含凭据而已脱敏]"));
    }
}
