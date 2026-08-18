mod application;
mod bundled_catalog;
mod error;
mod logging;
mod model_mutation;
mod models_write;
mod omp_environment;
mod overview;
mod provider_mutation;
mod redaction;
mod role_mutation;
mod target_configuration;

use application::{
    AppService, confirm_selected_omp, create_custom_provider, create_model, delete_model,
    detect_omp, edit_custom_provider, edit_model, get_overview_load, get_startup_state,
    get_ui_settings, initialize_target_configuration, open_target_configuration_directory,
    save_model_roles, save_ui_settings, validate_selected_omp,
};

use tauri::Manager;

#[cfg(test)]
#[path = "../manifest_registry_build.rs"]
mod manifest_registry_build;

#[cfg(test)]
mod application_test;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init().expect("failed to initialize redacted application logging");
    tracing::info!("starting OMP Switch without configuration payload logging");

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let settings_path = app.path().app_data_dir()?.join("settings.json");
            let service = AppService::new(settings_path)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_startup_state,
            get_overview_load,
            detect_omp,
            validate_selected_omp,
            confirm_selected_omp,
            initialize_target_configuration,
            open_target_configuration_directory,
            get_ui_settings,
            save_ui_settings,
            create_custom_provider,
            edit_custom_provider,
            create_model,
            edit_model,
            delete_model,
            save_model_roles,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run OMP Switch");
}
