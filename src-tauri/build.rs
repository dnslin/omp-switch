use std::{env, fs, path::PathBuf};

mod manifest_registry_build;

fn main() {
    let manifest_dir = PathBuf::from("resources/bundled-manifests");
    println!("cargo:rerun-if-changed={}", manifest_dir.display());
    println!("cargo:rerun-if-changed=capabilities");
    let registry =
        manifest_registry_build::build(&manifest_dir).unwrap_or_else(|error| panic!("{error}"));
    for path in &registry.manifest_paths {
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let output = PathBuf::from(env::var("OUT_DIR").expect("Cargo did not provide OUT_DIR"))
        .join("bundled_manifest_registry.rs");
    fs::write(output, registry.source).expect("could not write bundled manifest registry");

    let capability_pattern = if env::var_os("CARGO_FEATURE_WEBDRIVER").is_some() {
        "./capabilities/**/*"
    } else {
        "./capabilities/main.json"
    };
    tauri_build::try_build(
        tauri_build::Attributes::new().capabilities_path_pattern(capability_pattern),
    )
    .expect("failed to prepare Tauri build");
}
