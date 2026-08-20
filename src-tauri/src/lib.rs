mod application;
mod bundled_catalog;
mod configuration_transaction;
mod error;
mod logging;
mod model_mutation;
mod model_test;
mod models_write;
mod omp_environment;
mod overview;
mod provider_mutation;
mod redaction;
mod role_mutation;
mod target_configuration;

#[cfg(feature = "webdriver")]
use application::set_webdriver_model_test_state;
use application::{
    AppService, accept_model_test_cost_notice, cancel_model_test, confirm_path_omp,
    confirm_selected_omp, create_custom_provider, create_model, delete_model, delete_provider,
    detect_omp, edit_custom_provider, edit_model, get_model_test_state, get_overview_load,
    get_runtime_info, get_settings_directories, get_startup_state, get_ui_settings,
    initialize_target_configuration, open_application_configuration_directory,
    open_application_log_directory, open_current_target_configuration_directory,
    open_target_backup_directory, open_target_configuration_directory, reset_ui_settings,
    save_model_roles, save_ui_settings, test_model, validate_path_omp, validate_selected_omp,
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

    let builder = tauri::Builder::default();
    #[cfg(feature = "webdriver")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());
    builder
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
            get_runtime_info,
            detect_omp,
            validate_selected_omp,
            validate_path_omp,
            confirm_selected_omp,
            confirm_path_omp,
            initialize_target_configuration,
            open_target_configuration_directory,
            open_current_target_configuration_directory,
            open_application_configuration_directory,
            open_application_log_directory,
            open_target_backup_directory,
            get_ui_settings,
            get_settings_directories,
            save_ui_settings,
            reset_ui_settings,
            accept_model_test_cost_notice,
            create_custom_provider,
            edit_custom_provider,
            delete_provider,
            create_model,
            edit_model,
            delete_model,
            save_model_roles,
            test_model,
            cancel_model_test,
            get_model_test_state,
            #[cfg(feature = "webdriver")]
            set_webdriver_model_test_state,
        ])
        .run(tauri::generate_context!())
        .expect("failed to run OMP Switch");
}
