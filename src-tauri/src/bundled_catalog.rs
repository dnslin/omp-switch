use std::{
    collections::{HashMap, HashSet},
    sync::LazyLock,
};

use serde::Deserialize;

use crate::error::AppError;

mod generated {
    include!(concat!(env!("OUT_DIR"), "/bundled_manifest_registry.rs"));
}

#[derive(Clone, Debug)]
pub(crate) struct BundledCatalog {
    provider_ids: HashSet<String>,
    model_ids: HashSet<(String, String)>,
}

#[derive(Debug, Deserialize)]
struct BundledManifest {
    version: String,
    providers: std::collections::BTreeMap<String, Vec<String>>,
}

static CATALOGS: LazyLock<Result<HashMap<String, BundledCatalog>, String>> =
    LazyLock::new(parse_manifests);

pub(crate) fn for_version(version: &str) -> Result<Option<&'static BundledCatalog>, AppError> {
    CATALOGS
        .as_ref()
        .map(|catalogs| catalogs.get(normalize_version(version)))
        .map_err(|error| AppError::internal(format!("内置 Provider 清单无法加载：{error}")))
}

impl BundledCatalog {
    pub(crate) fn contains_provider(&self, provider_id: &str) -> bool {
        self.provider_ids.contains(&normalize_id(provider_id))
    }

    pub(crate) fn contains_model(&self, provider_id: &str, model_id: &str) -> bool {
        self.model_ids
            .contains(&(normalize_id(provider_id), normalize_id(model_id)))
    }
}

fn parse_manifests() -> Result<HashMap<String, BundledCatalog>, String> {
    let mut catalogs = HashMap::with_capacity(generated::BUNDLED_MANIFESTS.len());
    for (expected_version, source) in generated::BUNDLED_MANIFESTS {
        let manifest = serde_json::from_str::<BundledManifest>(source).map_err(|error| {
            format!("{expected_version} bundled manifest is invalid JSON: {error}")
        })?;
        if manifest.version != *expected_version {
            return Err(format!(
                "manifest version {} does not match {}",
                manifest.version, expected_version
            ));
        }
        let catalog = BundledCatalog::from_manifest(manifest);
        if catalogs
            .insert((*expected_version).to_owned(), catalog)
            .is_some()
        {
            return Err(format!(
                "duplicate bundled manifest version {expected_version}"
            ));
        }
    }
    if catalogs.is_empty() {
        return Err("no bundled Provider manifests were compiled".to_owned());
    }
    Ok(catalogs)
}

impl BundledCatalog {
    fn from_manifest(manifest: BundledManifest) -> Self {
        let mut provider_ids = HashSet::new();
        let mut model_ids = HashSet::new();
        for (provider_id, models) in manifest.providers {
            let provider_id = normalize_id(&provider_id);
            provider_ids.insert(provider_id.clone());
            for model_id in models {
                model_ids.insert((provider_id.clone(), normalize_id(&model_id)));
            }
        }
        Self {
            provider_ids,
            model_ids,
        }
    }
}

fn normalize_version(version: &str) -> &str {
    let trimmed = version.trim();
    trimmed
        .strip_prefix("omp/")
        .or_else(|| trimmed.strip_prefix('v'))
        .unwrap_or(trimmed)
}

pub(crate) fn normalize_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::for_version;

    #[test]
    fn loads_an_exact_generated_bundled_version_case_insensitively() {
        let catalog = for_version("omp/17.2.15").unwrap().unwrap();
        assert!(catalog.contains_provider("OpenAI"));
        assert!(catalog.contains_model("OPENAI", "GPT-5.6-SOL"));
    }

    #[test]
    fn does_not_infer_unlisted_versions() {
        assert!(for_version("17.2").unwrap().is_none());
        assert!(for_version("18.1.0").unwrap().is_none());
    }
}
