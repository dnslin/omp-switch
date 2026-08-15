use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
};

#[derive(Debug)]
pub(crate) struct ManifestRegistry {
    pub(crate) source: String,
    pub(crate) manifest_paths: Vec<PathBuf>,
}

pub(crate) fn build(manifest_dir: &Path) -> Result<ManifestRegistry, String> {
    let mut manifest_paths = fs::read_dir(manifest_dir)
        .map_err(|error| format!("could not read bundled manifest directory: {error}"))?
        .map(|entry| {
            entry.map(|entry| entry.path()).map_err(|error| {
                format!("could not read bundled manifest directory entry: {error}")
            })
        })
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    manifest_paths.sort();

    let mut versions = BTreeSet::new();
    let mut source = String::from("pub(crate) const BUNDLED_MANIFESTS: &[(&str, &str)] = &[\n");
    for path in &manifest_paths {
        let expected_version = path
            .file_stem()
            .and_then(|value| value.to_str())
            .ok_or_else(|| {
                format!(
                    "bundled manifest path is not valid UTF-8: {}",
                    path.display()
                )
            })?;
        if !versions.insert(expected_version.to_owned()) {
            return Err(format!(
                "duplicate bundled manifest version: {expected_version}"
            ));
        }

        let manifest_source = fs::read_to_string(path).map_err(|error| {
            format!(
                "could not read bundled manifest {}: {error}",
                path.display()
            )
        })?;
        validate_manifest(&manifest_source, expected_version, path)?;
        source.push_str(&format!(
            "    ({expected_version:?}, {manifest_source:?}),\n"
        ));
    }
    if versions.is_empty() {
        return Err("at least one bundled Provider manifest is required".to_owned());
    }
    source.push_str("];\n");

    Ok(ManifestRegistry {
        source,
        manifest_paths,
    })
}

fn validate_manifest(source: &str, expected_version: &str, path: &Path) -> Result<(), String> {
    let manifest = serde_json::from_str::<serde_json::Value>(source).map_err(|error| {
        format!(
            "bundled manifest {} is invalid JSON: {error}",
            path.display()
        )
    })?;
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| format!("bundled manifest {} has no string version", path.display()))?;
    if version != expected_version {
        return Err(format!(
            "bundled manifest {} declares {version}, expected {expected_version}",
            path.display()
        ));
    }
    let providers = manifest
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| format!("bundled manifest {} has no provider object", path.display()))?;
    for (provider_id, models) in providers {
        let models = models.as_array().ok_or_else(|| {
            format!(
                "bundled manifest {} has a non-array model list for {provider_id}",
                path.display()
            )
        })?;
        if models.iter().any(|model| !model.is_string()) {
            return Err(format!(
                "bundled manifest {} has a non-string model ID for {provider_id}",
                path.display()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use tempfile::tempdir;

    use super::build;

    fn write_manifest(directory: &Path, name: &str, source: &str) {
        fs::write(directory.join(name), source).unwrap();
    }

    #[test]
    fn compiles_each_valid_manifest_into_one_registry() {
        let directory = tempdir().unwrap();
        write_manifest(
            directory.path(),
            "17.2.15.json",
            r#"{"version":"17.2.15","providers":{"openai":["gpt"]}}"#,
        );
        write_manifest(
            directory.path(),
            "18.0.0.json",
            r#"{"version":"18.0.0","providers":{"anthropic":["claude"]}}"#,
        );

        let registry = build(directory.path()).unwrap();

        assert_eq!(registry.manifest_paths.len(), 2);
        let first = registry.source.find("(\"17.2.15\",").unwrap();
        let second = registry.source.find("(\"18.0.0\",").unwrap();
        assert!(first < second);
    }

    #[test]
    fn rejects_a_manifest_with_a_mismatched_file_version() {
        let directory = tempdir().unwrap();
        write_manifest(
            directory.path(),
            "17.2.15.json",
            r#"{"version":"17.2.16","providers":{}}"#,
        );

        let error = build(directory.path()).unwrap_err();

        assert!(error.contains("declares 17.2.16, expected 17.2.15"));
    }

    #[test]
    fn rejects_invalid_manifest_json() {
        let directory = tempdir().unwrap();
        write_manifest(directory.path(), "17.2.15.json", "{not json");

        let error = build(directory.path()).unwrap_err();

        assert!(error.contains("is invalid JSON"), "{error}");
    }

    #[test]
    fn rejects_an_empty_manifest_directory() {
        let directory = tempdir().unwrap();

        let error = build(directory.path()).unwrap_err();

        assert_eq!(error, "at least one bundled Provider manifest is required");
    }

    #[test]
    fn rejects_invalid_provider_and_model_shapes() {
        for (source, expected_error) in [
            (
                r#"{"version":"17.2.15","providers":[]}"#,
                "has no provider object",
            ),
            (
                r#"{"version":"17.2.15","providers":{"openai":"gpt"}}"#,
                "has a non-array model list for openai",
            ),
            (
                r#"{"version":"17.2.15","providers":{"openai":[42]}}"#,
                "has a non-string model ID for openai",
            ),
        ] {
            let directory = tempdir().unwrap();
            write_manifest(directory.path(), "17.2.15.json", source);

            let error = build(directory.path()).unwrap_err();

            assert!(error.contains(expected_error), "{error}");
        }
    }
}
