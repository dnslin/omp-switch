use std::{collections::HashSet, sync::LazyLock};

use serde::Deserialize;

use crate::error::AppError;

const SUPPORTED_MANIFEST_VERSION: &str = "17.2.15";
const SUPPORTED_MANIFEST: &str = include_str!("../resources/bundled-manifests/17.2.15.json");

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

static CATALOG: LazyLock<Result<BundledCatalog, String>> = LazyLock::new(parse_manifest);

pub(crate) fn for_version(version: &str) -> Result<Option<&'static BundledCatalog>, AppError> {
    if normalize_version(version) != SUPPORTED_MANIFEST_VERSION {
        return Ok(None);
    }
    CATALOG
        .as_ref()
        .map(Some)
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

fn parse_manifest() -> Result<BundledCatalog, String> {
    let manifest = serde_json::from_str::<BundledManifest>(SUPPORTED_MANIFEST)
        .map_err(|error| error.to_string())?;
    if manifest.version != SUPPORTED_MANIFEST_VERSION {
        return Err(format!(
            "manifest version {} does not match {}",
            manifest.version, SUPPORTED_MANIFEST_VERSION
        ));
    }

    let mut provider_ids = HashSet::new();
    let mut model_ids = HashSet::new();
    for (provider_id, models) in manifest.providers {
        let provider_id = normalize_id(&provider_id);
        provider_ids.insert(provider_id.clone());
        for model_id in models {
            model_ids.insert((provider_id.clone(), normalize_id(&model_id)));
        }
    }
    Ok(BundledCatalog {
        provider_ids,
        model_ids,
    })
}

fn normalize_version(version: &str) -> &str {
    let trimmed = version.trim();
    trimmed
        .strip_prefix("omp/")
        .or_else(|| trimmed.strip_prefix('v'))
        .unwrap_or(trimmed)
}

fn normalize_id(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::for_version;

    #[test]
    fn loads_the_exact_bundled_version_case_insensitively() {
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
