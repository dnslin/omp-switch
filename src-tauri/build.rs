use std::{collections::BTreeSet, env, fs, path::PathBuf};

fn main() {
    let manifest_dir = PathBuf::from("resources/bundled-manifests");
    println!("cargo:rerun-if-changed={}", manifest_dir.display());

    let mut manifest_paths = fs::read_dir(&manifest_dir)
        .unwrap_or_else(|error| panic!("could not read bundled manifest directory: {error}"))
        .map(|entry| {
            entry
                .expect("could not read bundled manifest directory entry")
                .path()
        })
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    manifest_paths.sort();

    let mut versions = BTreeSet::new();
    let mut registry = String::from("pub(crate) const BUNDLED_MANIFESTS: &[(&str, &str)] = &[\n");
    for path in manifest_paths {
        println!("cargo:rerun-if-changed={}", path.display());
        let expected_version = path
            .file_stem()
            .and_then(|value| value.to_str())
            .unwrap_or_else(|| {
                panic!(
                    "bundled manifest path is not valid UTF-8: {}",
                    path.display()
                )
            });
        if !versions.insert(expected_version.to_owned()) {
            panic!("duplicate bundled manifest version: {expected_version}");
        }

        let source = fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!(
                "could not read bundled manifest {}: {error}",
                path.display()
            )
        });
        validate_manifest(&source, expected_version, &path);
        registry.push_str(&format!("    ({expected_version:?}, {source:?}),\n"));
    }
    if versions.is_empty() {
        panic!("at least one bundled Provider manifest is required");
    }
    registry.push_str("];\n");

    let output = PathBuf::from(env::var("OUT_DIR").expect("Cargo did not provide OUT_DIR"))
        .join("bundled_manifest_registry.rs");
    fs::write(output, registry).expect("could not write bundled manifest registry");
    tauri_build::build()
}

fn validate_manifest(source: &str, expected_version: &str, path: &std::path::Path) {
    let manifest = serde_json::from_str::<serde_json::Value>(source).unwrap_or_else(|error| {
        panic!(
            "bundled manifest {} is invalid JSON: {error}",
            path.display()
        )
    });
    let version = manifest
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("bundled manifest {} has no string version", path.display()));
    if version != expected_version {
        panic!(
            "bundled manifest {} declares {version}, expected {expected_version}",
            path.display()
        );
    }
    let providers = manifest
        .get("providers")
        .and_then(serde_json::Value::as_object)
        .unwrap_or_else(|| panic!("bundled manifest {} has no provider object", path.display()));
    for (provider_id, models) in providers {
        let models = models.as_array().unwrap_or_else(|| {
            panic!(
                "bundled manifest {} has a non-array model list for {provider_id}",
                path.display()
            )
        });
        if models.iter().any(|model| !model.is_string()) {
            panic!(
                "bundled manifest {} has a non-string model ID for {provider_id}",
                path.display()
            );
        }
    }
}
