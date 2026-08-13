mod application;
mod error;
mod logging;

use application::{
    AppService, confirm_selected_omp, detect_omp, get_startup_state, get_ui_settings,
    save_ui_settings, validate_selected_omp,
};
use tauri::Manager;

#[cfg(test)]
mod application_test;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init().expect("failed to initialize redacted application logging");
    tracing::info!("starting OMP Switch without configuration payload logging");

    tauri::Builder::default()
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
            detect_omp,
            validate_selected_omp,
            confirm_selected_omp,
            get_ui_settings,
            save_ui_settings
        ])
        .run(tauri::generate_context!())
        .expect("failed to run OMP Switch");
}
