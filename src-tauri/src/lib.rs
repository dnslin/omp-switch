mod application;
mod error;
mod logging;

use application::{AppService, get_startup_state, get_ui_settings, save_ui_settings};
use tauri::Manager;

#[cfg(test)]
mod application_test;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    logging::init();
    tracing::info!("starting OMP Switch without configuration payload logging");

    tauri::Builder::default()
        .setup(|app| {
            let settings_path = app.path().app_data_dir()?.join("settings.json");
            let service = AppService::new(settings_path)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            app.manage(service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_startup_state,
            get_ui_settings,
            save_ui_settings
        ])
        .run(tauri::generate_context!())
        .expect("failed to run OMP Switch");
}
