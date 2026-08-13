use std::fs;

use pretty_assertions::assert_eq;
use tempfile::tempdir;

use crate::application::{AppService, AppSettings, StartupState, Theme};

#[test]
fn intent_command_returns_secret_free_startup_dto() {
    let app_data = tempdir().unwrap();
    let service = AppService::new(app_data.path().join("settings.json")).unwrap();

    assert_eq!(
        service.get_startup_state(),
        StartupState::OmpUnavailable {
            message: "尚未检测 OMP".to_owned(),
        }
    );
}

#[test]
fn settings_seam_persists_only_approved_lightweight_state() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    let service = AppService::new(settings_path.clone()).unwrap();
    let settings = AppSettings {
        omp_executable_path: Some("/usr/local/bin/omp".to_owned()),
        theme: Theme::Dark,
        selected_provider_id: Some("dnslin".to_owned()),
        selected_model_id: Some("gpt-5.6-sol".to_owned()),
        cost_notice_accepted: true,
    };

    assert_eq!(
        service.save_ui_settings(settings.clone()).unwrap(),
        settings
    );

    let rebuilt = AppService::new(settings_path).unwrap();
    assert_eq!(rebuilt.get_ui_settings().unwrap(), settings);
    let persisted = fs::read_to_string(app_data.path().join("settings.json")).unwrap();
    assert!(!persisted.contains("apiKey"));
    assert!(!persisted.contains("modelRoles"));
}
