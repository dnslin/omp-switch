use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use serde_yaml::{Mapping, Value};

use crate::{
    bundled_catalog::BundledCatalog,
    error::AppError,
    models_write::{self, ModelsMutation, ModelsWriteFailurePoint},
    overview,
    redaction::redact_diagnostic,
    target_configuration::TargetConfigurationDiscovery,
};

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
    Auto,
}

impl ThinkingLevel {
    fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
            Self::Auto => "auto",
        }
    }
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub(crate) enum ModelRoleChange {
    Set {
        #[serde(rename = "roleId")]
        role_id: String,
        #[serde(rename = "providerId")]
        provider_id: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: Option<ThinkingLevel>,
    },
    Create {
        #[serde(rename = "roleId")]
        role_id: String,
        #[serde(rename = "providerId")]
        provider_id: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: Option<ThinkingLevel>,
    },
    Rename {
        #[serde(rename = "roleId")]
        role_id: String,
        #[serde(rename = "newRoleId")]
        new_role_id: String,
        #[serde(rename = "providerId")]
        provider_id: String,
        #[serde(rename = "modelId")]
        model_id: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: Option<ThinkingLevel>,
    },
    Clear {
        #[serde(rename = "roleId")]
        role_id: String,
    },
    Delete {
        #[serde(rename = "roleId")]
        role_id: String,
    },
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SaveModelRolesInput {
    pub(crate) opened_config_hash: String,
    pub(crate) changes: Vec<ModelRoleChange>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SaveModelRolesResult {
    pub(crate) changed_role_count: usize,
}

#[derive(Clone)]
enum ValidatedRoleChange {
    Set {
        role_id: String,
        selector: String,
    },
    Rename {
        role_id: String,
        new_role_id: String,
        selector: String,
    },
    Clear {
        role_id: String,
    },
}

struct ValidatedSave {
    changes: Vec<ValidatedRoleChange>,
    models_path: PathBuf,
    models_hash: String,
    catalog: Option<BundledCatalog>,
}

impl ModelsMutation for ValidatedSave {
    fn verb(&self) -> &'static str {
        "保存模型角色"
    }

    fn serialization_error(&self) -> (&'static str, &'static str, &'static str) {
        (
            "roles-serialize-error",
            "无法序列化模型角色修改",
            "请检查角色输入后重试；原 config.yml 没有被修改。",
        )
    }

    fn apply(&self, tree: &mut Value) -> Result<(), AppError> {
        let roles = model_roles_mut(tree)?;
        for change in &self.changes {
            match change {
                ValidatedRoleChange::Set { role_id, selector } => {
                    roles.insert(key(role_id), Value::String(selector.clone()));
                }
                ValidatedRoleChange::Rename {
                    role_id,
                    new_role_id,
                    selector,
                } => {
                    roles.remove(key(role_id));
                    roles.insert(key(new_role_id), Value::String(selector.clone()));
                }
                ValidatedRoleChange::Clear { role_id } => {
                    roles.remove(key(role_id));
                }
            }
        }
        Ok(())
    }

    fn validate(&self, candidate: &Value, original: &Value) -> Result<(), AppError> {
        let original_root = config_root(original)?;
        let candidate_root = config_root(candidate)?;
        if original_root.len() != candidate_root.len() {
            return Err(roles_untouched_error());
        }
        for (field, value) in original_root {
            if field.as_str() != Some("modelRoles") && candidate_root.get(field) != Some(value) {
                return Err(roles_untouched_error());
            }
        }
        let mut expected_roles = model_roles(original)?.clone();
        for change in &self.changes {
            match change {
                ValidatedRoleChange::Set { role_id, selector } => {
                    expected_roles.insert(key(role_id), Value::String(selector.clone()));
                }
                ValidatedRoleChange::Rename {
                    role_id,
                    new_role_id,
                    selector,
                } => {
                    expected_roles.remove(key(role_id));
                    expected_roles.insert(key(new_role_id), Value::String(selector.clone()));
                }
                ValidatedRoleChange::Clear { role_id } => {
                    expected_roles.remove(key(role_id));
                }
            }
        }
        if candidate_root.get(key("modelRoles")) != Some(&Value::Mapping(expected_roles)) {
            return Err(roles_untouched_error());
        }
        Ok(())
    }
    fn validate_before_commit(&self, loaded: &models_write::LoadedModels) -> Result<(), AppError> {
        models_write::ensure_resolved_file_path(
            &self.models_path,
            &loaded.expected_target,
            "models.yml",
        )?;
        let bytes = fs::read(&self.models_path).map_err(|error| {
            AppError::new(
                "role-model-hash-conflict",
                format!("无法重新读取 models.yml：{}", error.kind()),
                "请重新读取配置；当前未保存角色修改已保留。",
            )
        })?;
        if models_write::content_hash(&bytes) != self.models_hash {
            return Err(role_models_changed());
        }
        let Some(catalog) = self.catalog.as_ref() else {
            return Ok(());
        };
        let models_tree = serde_yaml::from_slice::<Value>(&bytes).map_err(|error| {
            AppError::new(
                "role-model-hash-conflict",
                format!(
                    "models.yml 无法重新解析：{}",
                    redact_diagnostic(&error.to_string())
                ),
                "请重新读取配置；当前未保存角色修改已保留。",
            )
        })?;
        for selector in self
            .changes
            .iter()
            .filter_map(ValidatedRoleChange::selector)
        {
            let Some((provider_id, model_id)) = selector_model_ids(selector) else {
                return Err(role_model_unavailable(selector));
            };
            if !overview::is_assignable_model_definition(
                &models_tree,
                provider_id,
                model_id,
                catalog,
            ) {
                return Err(role_model_unavailable(selector));
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn mutate_external_state_for_test(&self) {
        let _ = fs::write(&self.models_path, "providers: {}\n");
    }
}

impl ValidatedRoleChange {
    fn selector(&self) -> Option<&str> {
        match self {
            Self::Set { selector, .. } | Self::Rename { selector, .. } => Some(selector),
            Self::Clear { .. } => None,
        }
    }
}

struct LoadedRoleModels {
    path: PathBuf,
    hash: String,
    tree: Value,
}

fn selector_parts(selector: &str) -> Option<(&str, &str, Option<&str>)> {
    let (provider_id, model_with_thinking) = overview::parse_role_selector(selector)?;
    let (model_id, thinking_level) = model_with_thinking
        .rsplit_once(':')
        .filter(|(_, thinking)| is_supported_thinking(thinking))
        .map_or((model_with_thinking, None), |(model, thinking)| {
            (model, Some(thinking))
        });
    Some((provider_id, model_id, thinking_level))
}

fn selector_model_ids(selector: &str) -> Option<(&str, &str)> {
    selector_parts(selector).map(|(provider_id, model_id, _)| (provider_id, model_id))
}
fn is_supported_thinking(value: &str) -> bool {
    matches!(
        value,
        "off" | "minimal" | "low" | "medium" | "high" | "xhigh" | "max" | "auto"
    )
}

pub(crate) fn save_model_roles(
    target: &TargetConfigurationDiscovery,
    backup_root: &Path,
    catalog: Option<&BundledCatalog>,
    input: &SaveModelRolesInput,
    failure: Option<ModelsWriteFailurePoint>,
) -> Result<SaveModelRolesResult, AppError> {
    let loaded = models_write::load_config_for_write(target, &input.opened_config_hash)
        .map_err(remap_role_error)?;
    let models = load_models_tree(target).map_err(remap_role_error)?;
    let validated = validate_input(
        input,
        &loaded.original_tree,
        &models.tree,
        catalog,
        &models.path,
        &models.hash,
    )?;
    let result = SaveModelRolesResult {
        changed_role_count: validated.changes.len(),
    };
    models_write::write_models_mutation(backup_root, &loaded, &validated, failure)
        .map_err(remap_role_error)?;
    Ok(result)
}

fn validate_input(
    input: &SaveModelRolesInput,
    config_tree: &Value,
    models_tree: &Value,
    catalog: Option<&BundledCatalog>,
    models_path: &Path,
    models_hash: &str,
) -> Result<ValidatedSave, AppError> {
    let roles = model_roles(config_tree)?;
    if overview::model_roles_have_advanced(config_tree, models_tree, catalog) {
        return Err(AppError::new(
            "role-advanced-read-only",
            "config.yml 中存在当前版本不支持的高级模型角色配置。",
            "请在 OMP 或外部编辑器中处理高级角色后重新读取；OMP Switch 不会部分覆盖角色语义。",
        ));
    }
    let mut changed_ids = HashSet::new();
    let mut changes = Vec::with_capacity(input.changes.len());
    for change in &input.changes {
        match change {
            ModelRoleChange::Set {
                role_id,
                provider_id,
                model_id,
                thinking_level,
            } => {
                let role_id = normalize_role_id(role_id)?;
                if !changed_ids.insert(role_id.clone()) {
                    return Err(duplicate_role_change());
                }
                if !is_builtin_role(&role_id) && !roles.contains_key(key(&role_id)) {
                    return Err(role_not_found(&role_id));
                }
                let selector = validate_selector(
                    provider_id,
                    model_id,
                    *thinking_level,
                    models_tree,
                    catalog,
                )?;
                changes.push(ValidatedRoleChange::Set { role_id, selector });
            }
            ModelRoleChange::Create {
                role_id,
                provider_id,
                model_id,
                thinking_level,
            } => {
                let role_id = normalize_role_id(role_id)?;
                if !changed_ids.insert(role_id.clone()) {
                    return Err(duplicate_role_change());
                }
                if is_builtin_role(&role_id) || roles.contains_key(key(&role_id)) {
                    return Err(role_conflict(&role_id));
                }
                let selector = validate_selector(
                    provider_id,
                    model_id,
                    *thinking_level,
                    models_tree,
                    catalog,
                )?;
                changes.push(ValidatedRoleChange::Set { role_id, selector });
            }
            ModelRoleChange::Rename {
                role_id,
                new_role_id,
                provider_id,
                model_id,
                thinking_level,
            } => {
                let role_id = normalize_role_id(role_id)?;
                let new_role_id = normalize_role_id(new_role_id)?;
                if role_id == new_role_id {
                    return Err(role_rename_same());
                }
                if !changed_ids.insert(role_id.clone()) || !changed_ids.insert(new_role_id.clone())
                {
                    return Err(duplicate_role_change());
                }
                if is_builtin_role(&role_id) {
                    return Err(role_builtin_immutable());
                }
                if !roles.contains_key(key(&role_id)) {
                    return Err(role_not_found(&role_id));
                }
                if is_builtin_role(&new_role_id) || roles.contains_key(key(&new_role_id)) {
                    return Err(role_conflict(&new_role_id));
                }
                let selector = validate_selector(
                    provider_id,
                    model_id,
                    *thinking_level,
                    models_tree,
                    catalog,
                )?;
                changes.push(ValidatedRoleChange::Rename {
                    role_id,
                    new_role_id,
                    selector,
                });
            }
            ModelRoleChange::Clear { role_id } => {
                let role_id = normalize_role_id(role_id)?;
                if !changed_ids.insert(role_id.clone()) {
                    return Err(duplicate_role_change());
                }
                if !is_builtin_role(&role_id) && !roles.contains_key(key(&role_id)) {
                    return Err(role_not_found(&role_id));
                }
                changes.push(ValidatedRoleChange::Clear { role_id });
            }
            ModelRoleChange::Delete { role_id } => {
                let role_id = normalize_role_id(role_id)?;
                if !changed_ids.insert(role_id.clone()) {
                    return Err(duplicate_role_change());
                }
                if is_builtin_role(&role_id) {
                    return Err(role_builtin_immutable());
                }
                if !roles.contains_key(key(&role_id)) {
                    return Err(role_not_found(&role_id));
                }
                changes.push(ValidatedRoleChange::Clear { role_id });
            }
        }
    }
    Ok(ValidatedSave {
        changes,
        models_path: models_path.to_owned(),
        models_hash: models_hash.to_owned(),
        catalog: catalog.cloned(),
    })
}

fn load_models_tree(target: &TargetConfigurationDiscovery) -> Result<LoadedRoleModels, AppError> {
    let path = models_write::resolved_path(&target.models.resolved_path, "models.yml")?;
    let expected_target =
        models_write::resolved_path(&target.resolved_path, "Target configuration")?;
    models_write::ensure_resolved_file_path(&path, &expected_target, "models.yml")?;
    let bytes = fs::read(&path).map_err(|error| {
        AppError::new(
            "role-models-unavailable",
            format!("无法读取 models.yml：{}", error.kind()),
            "请重新读取配置后重试；角色选择不会覆盖无法验证的 Model definition。",
        )
    })?;
    let hash = models_write::content_hash(&bytes);
    let tree = serde_yaml::from_slice(&bytes).map_err(|error| {
        AppError::new(
            "role-models-parse-error",
            format!(
                "models.yml 无法重新解析：{}",
                redact_diagnostic(&error.to_string())
            ),
            "请在外部修复 YAML 后重新读取；角色选择不会覆盖错误文件。",
        )
    })?;
    Ok(LoadedRoleModels { path, hash, tree })
}

fn validate_selector(
    provider_id: &str,
    model_id: &str,
    thinking_level: Option<ThinkingLevel>,
    models_tree: &Value,
    catalog: Option<&BundledCatalog>,
) -> Result<String, AppError> {
    let mut selector = format!("{provider_id}/{model_id}");
    if let Some(level) = thinking_level {
        selector.push(':');
        selector.push_str(level.as_str());
    }
    let Some((parsed_provider_id, parsed_model_id, parsed_thinking_level)) =
        selector_parts(&selector)
    else {
        return Err(AppError::new(
            "role-selector-invalid",
            "Provider/Model definition 不能安全表示为 Simple role selector。",
            "请选择不含空白、控制字符、逗号且不以 @ 开头的 Provider/Model definition。",
        ));
    };
    if parsed_provider_id != provider_id
        || parsed_model_id != model_id
        || parsed_thinking_level != thinking_level.map(ThinkingLevel::as_str)
    {
        return Err(AppError::new(
            "role-selector-invalid",
            "Provider/Model definition 不能无歧义表示为 Simple role selector。",
            "请选择不含 / 或 Thinking 后缀歧义的 Provider/Model definition。",
        ));
    }
    let catalog = catalog.ok_or_else(|| {
        AppError::new(
            "role-catalog-unavailable",
            "当前 OMP 版本没有匹配的 bundled Provider 清单。",
            "请重新检测支持的 OMP 版本后再选择 Model role。",
        )
    })?;
    if !overview::is_assignable_model_definition(models_tree, provider_id, model_id, catalog) {
        return Err(AppError::new(
            "role-model-unavailable",
            format!("无法选择 {provider_id}/{model_id}。"),
            "请选择普通、完整且当前可用的 Provider/Model definition。",
        ));
    }
    Ok(selector)
}
fn normalize_role_id(value: &str) -> Result<String, AppError> {
    if !overview::is_valid_role_id(value) {
        return Err(AppError::new(
            "role-id-invalid",
            "模型角色名称不能为空，且不能包含空白、/、逗号或控制字符。",
            "请使用不含特殊字符的角色名称。",
        ));
    }
    Ok(value.to_owned())
}

fn model_roles(tree: &Value) -> Result<&Mapping, AppError> {
    config_root(tree)?
        .get(key("modelRoles"))
        .and_then(Value::as_mapping)
        .ok_or_else(roles_structure_error)
}

fn model_roles_mut(tree: &mut Value) -> Result<&mut Mapping, AppError> {
    tree.as_mapping_mut()
        .and_then(|root| root.get_mut(key("modelRoles")))
        .and_then(Value::as_mapping_mut)
        .ok_or_else(roles_structure_error)
}

fn config_root(tree: &Value) -> Result<&Mapping, AppError> {
    tree.as_mapping().ok_or_else(roles_structure_error)
}

fn is_builtin_role(role_id: &str) -> bool {
    matches!(
        role_id,
        "default"
            | "smol"
            | "slow"
            | "vision"
            | "plan"
            | "designer"
            | "commit"
            | "tiny"
            | "task"
            | "advisor"
    )
}

fn key(value: &str) -> Value {
    Value::String(value.to_owned())
}

fn roles_structure_error() -> AppError {
    AppError::new(
        "role-config-structure-invalid",
        "config.yml 的 modelRoles 结构无法安全编辑。",
        "请在外部修复 modelRoles 后重新读取；OMP Switch 不会覆盖未知结构。",
    )
}

fn roles_untouched_error() -> AppError {
    AppError::new(
        "role-untouched-path-changed",
        "保存模型角色时检测到未触及配置路径发生变化。",
        "请重新读取配置；OMP Switch 不会覆盖其他 config.yml 设置。",
    )
}

fn duplicate_role_change() -> AppError {
    AppError::new(
        "role-change-duplicate",
        "同一个模型角色在一次保存中被修改了多次。",
        "请重新读取角色页后重试。",
    )
}

fn role_not_found(role_id: &str) -> AppError {
    AppError::new(
        "role-not-found",
        format!("模型角色 {role_id} 不存在。"),
        "请重新读取角色页后重试。",
    )
}

fn role_conflict(role_id: &str) -> AppError {
    AppError::new(
        "role-id-conflict",
        format!("模型角色 {role_id} 已存在或与内置角色重名。"),
        "请选择一个新的自定义角色名称。",
    )
}

fn role_rename_same() -> AppError {
    AppError::new(
        "role-rename-unchanged",
        "自定义角色名称没有变化。",
        "请输入新的自定义角色名称，或取消改名。",
    )
}

fn role_builtin_immutable() -> AppError {
    AppError::new(
        "role-builtin-immutable",
        "内置模型角色不能改名或删除。",
        "内置角色只能设置或清除模型选择器。",
    )
}

fn role_models_changed() -> AppError {
    AppError::new(
        "role-model-hash-conflict",
        "保存前检测到 models.yml 已被外部修改。",
        "请重新读取配置；当前未保存角色修改已保留，OMP Switch 不会自动合并。",
    )
}

fn role_model_unavailable(selector: &str) -> AppError {
    AppError::new(
        "role-model-unavailable",
        format!("保存前无法确认角色选择器 {selector} 仍指向普通、完整且可用的 Model definition。"),
        "请重新读取角色页后重试；当前未保存角色修改已保留。",
    )
}

fn remap_role_error(error: AppError) -> AppError {
    match error.code {
        "provider-create-unavailable" | "provider-create-target-changed" => AppError::new(
            "role-write-unavailable",
            "当前 config.yml 不允许安全保存模型角色。",
            "请重新检测 OMP，并按当前只读、迁移或错误提示处理。",
        ),
        "models-hash-conflict" => AppError::new(
            "config-hash-conflict",
            "config.yml 在打开角色页后已被外部修改。",
            "请重新读取配置；当前未保存角色修改已保留，OMP Switch 不会自动合并。",
        ),
        "provider-create-failed"
        | "models-serialize-error"
        | "models-temporary-parse-error"
        | "models-temporary-validation-error"
        | "models-untouched-path-changed" => AppError::new(
            "role-write-failed",
            "无法安全写入 config.yml。",
            "请检查路径、权限和可用磁盘空间后重试；原 config.yml 没有被修改。",
        ),
        "models-replacement-outcome-unknown" => AppError::new(
            "role-replacement-outcome-unknown",
            "无法确认 config.yml 是否已写入。",
            "请重新读取并核对角色值；在确认当前状态前不要重复保存。",
        ),
        _ => error,
    }
}
