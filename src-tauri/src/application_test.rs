use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::application::{
    AppService, AppSettings, CommandOutput, ConfigurationFileStatus, OmpEnvironment, StartupState,
    TargetAccess, Theme, UiSettingsUpdate,
};
use parking_lot::Mutex;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[derive(Default)]
struct FakeOmpEnvironment {
    path_omp: Option<PathBuf>,
    calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    inspect_target_error: Option<std::io::ErrorKind>,
}

impl FakeOmpEnvironment {
    fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path_omp: Some(path.into()),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.calls.lock().clone()
    }
}

impl OmpEnvironment for FakeOmpEnvironment {
    fn find_in_path(&self) -> Option<PathBuf> {
        self.path_omp.clone()
    }

    fn run(&self, executable: &Path, arguments: &[&str]) -> std::io::Result<CommandOutput> {
        self.calls.lock().push((
            executable.to_path_buf(),
            arguments.iter().map(|value| (*value).to_owned()).collect(),
        ));
        let name = executable.file_name().unwrap().to_string_lossy();
        match (name.as_ref(), arguments) {
            ("saved-omp", ["--version"]) => Ok(CommandOutput::success("17.4.1\n")),
            ("saved-omp", ["config", "path"]) => Ok(CommandOutput::success("/tmp/saved-agent\n")),
            ("path-omp", ["--version"]) => Ok(CommandOutput::success("18.0.0\n")),
            ("path-omp", ["config", "path"]) => Ok(CommandOutput::success("/tmp/path-agent\n")),
            ("broken-version", ["--version"]) => {
                Ok(CommandOutput::failure(7, "API_KEY=super-secret"))
            }
            ("relative-path", ["--version"]) => Ok(CommandOutput::success("17.4.1")),
            ("relative-path", ["config", "path"]) => Ok(CommandOutput::success("relative/agent")),
            ("noisy-path", ["--version"]) => Ok(CommandOutput::success("17.4.1")),
            ("noisy-path", ["config", "path"]) => Ok(CommandOutput::success("/tmp/one\n/tmp/two")),
            _ => Err(std::io::Error::new(std::io::ErrorKind::NotFound, "missing")),
        }
    }

    fn inspect_target(&self, _target: &Path) -> std::io::Result<TargetAccess> {
        if let Some(kind) = self.inspect_target_error {
            return Err(std::io::Error::new(kind, "target inspection failed"));
        }
        Ok(TargetAccess {
            writable: true,
            models_yml: ConfigurationFileStatus::Normal,
            config_yml: ConfigurationFileStatus::Normal,
        })
    }
}

fn service_with(environment: Arc<FakeOmpEnvironment>, saved: Option<&str>) -> AppService {
    let app_data = tempdir().unwrap().keep();
    let settings_path = app_data.join("settings.json");
    if let Some(path) = saved {
        fs::create_dir_all(&app_data).unwrap();
        fs::write(
            &settings_path,
            serde_json::to_vec_pretty(&AppSettings {
                omp_executable_path: Some(path.to_owned()),
                ..AppSettings::default()
            })
            .unwrap(),
        )
        .unwrap();
    }
    AppService::new_with_environment(settings_path, environment).unwrap()
}

#[test]
fn startup_detection_prefers_saved_omp_and_runs_only_fixed_commands() {
    let environment = Arc::new(FakeOmpEnvironment::with_path("/bin/path-omp"));
    let service = service_with(environment.clone(), Some("/bin/saved-omp"));

    assert_eq!(
        service.detect_omp(),
        StartupState::OmpReady {
            executable_path: "/bin/saved-omp".to_owned(),
            version: "17.4.1".to_owned(),
            target_configuration: "/tmp/saved-agent".to_owned(),
            target_access: TargetAccess {
                writable: true,
                models_yml: ConfigurationFileStatus::Normal,
                config_yml: ConfigurationFileStatus::Normal,
            },
            requires_confirmation: false,
            previous_target_configuration: None,
        }
    );
    assert_eq!(
        environment.calls(),
        vec![
            (
                PathBuf::from("/bin/saved-omp"),
                vec!["--version".to_owned()]
            ),
            (
                PathBuf::from("/bin/saved-omp"),
                vec!["config".to_owned(), "path".to_owned()],
            ),
        ]
    );
}

#[test]
fn startup_detection_requires_confirmation_before_replacing_unusable_saved_executable_with_path() {
    let environment = Arc::new(FakeOmpEnvironment::with_path("/bin/path-omp"));
    let service = service_with(environment.clone(), Some("/bin/missing-omp"));

    assert!(matches!(
        service.detect_omp(),
        StartupState::OmpReady { executable_path, requires_confirmation: true, .. }
            if executable_path == "/bin/path-omp"
    ));
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/missing-omp")
    );

    service
        .confirm_selected_omp(PathBuf::from("/bin/path-omp"))
        .unwrap();
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/path-omp")
    );
}

#[test]
fn version_failure_returns_redacted_diagnostics_and_exit_code() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, None);

    let state = service.validate_selected_omp(PathBuf::from("/bin/broken-version"));
    assert!(
        matches!(state, StartupState::VersionFailed { exit_code: Some(7), ref stderr, .. } if stderr.contains("已脱敏") && !stderr.contains("super-secret"))
    );
}

#[test]
fn config_path_must_be_one_absolute_directory() {
    for executable in ["/bin/relative-path", "/bin/noisy-path"] {
        let environment = Arc::new(FakeOmpEnvironment::default());
        let service = service_with(environment, None);
        assert!(matches!(
            service.validate_selected_omp(PathBuf::from(executable)),
            StartupState::ConfigPathFailed { .. }
        ));
    }
}

#[test]
fn failed_manual_replacement_keeps_current_saved_omp() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/saved-omp"));

    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/broken-version")),
        StartupState::VersionFailed { .. }
    ));
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/saved-omp")
    );
}

#[test]
fn valid_manual_replacement_is_saved_only_after_explicit_confirmation() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/saved-omp"));
    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
        StartupState::OmpReady {
            ref target_configuration,
            ref previous_target_configuration,
            ..
        } if target_configuration == "/tmp/path-agent"
            && previous_target_configuration.as_deref() == Some("/tmp/saved-agent")
    ));
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/saved-omp")
    );

    service
        .confirm_selected_omp(PathBuf::from("/bin/path-omp"))
        .unwrap();
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/path-omp")
    );
}

#[test]
fn failed_revalidation_clears_previous_pending_omp() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/saved-omp"));

    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
        StartupState::OmpReady { .. }
    ));
    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/broken-version")),
        StartupState::VersionFailed { .. }
    ));
    assert!(
        service
            .confirm_selected_omp(PathBuf::from("/bin/path-omp"))
            .is_err()
    );
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/saved-omp")
    );
}

#[test]
fn automatic_detection_clears_previous_manual_pending_omp() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/saved-omp"));

    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
        StartupState::OmpReady { .. }
    ));
    assert!(matches!(
        service.detect_omp(),
        StartupState::OmpReady {
            requires_confirmation: false,
            ..
        }
    ));
    assert!(
        service
            .confirm_selected_omp(PathBuf::from("/bin/path-omp"))
            .is_err()
    );
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/saved-omp")
    );
}

#[test]
fn diagnostics_redact_common_secret_assignments_and_authorization_headers() {
    let diagnostic = "API_KEY super-secret password=hunter2 Authorization: Bearer abc.def token 'quoted secret' client_secret: hidden access_token= spaced x-api-key: header-secret sk-live-raw safe-context";
    let redacted = crate::application::redact_diagnostic(diagnostic);
    for secret in [
        "super-secret",
        "hunter2",
        "abc.def",
        "quoted secret",
        "hidden",
        "spaced",
        "header-secret",
        "sk-live-raw",
    ] {
        assert!(
            !redacted.contains(secret),
            "secret {secret:?} leaked in {redacted:?}"
        );
    }
    assert!(redacted.contains("safe-context"));
    assert!(redacted.contains("[已脱敏]"));
}

#[test]
fn diagnostics_suppress_structured_json_and_url_credentials() {
    for diagnostic in [
        r#"request failed: {\"token\":\"json-secret\",\"message\":\"denied\"}"#,
        "request failed: https://example.test/models?api_key=query-secret&limit=10",
    ] {
        let redacted = crate::application::redact_diagnostic(diagnostic);
        assert_eq!(redacted, "[诊断信息因可能包含凭据而已脱敏]");
        assert!(!redacted.contains("json-secret"));
        assert!(!redacted.contains("query-secret"));
    }
}

#[test]
fn diagnostics_redact_provider_prefixed_secret_names() {
    let diagnostic = "OPENAI_API_KEY=sk-openai-live ANTHROPIC_API_KEY anthropic-live AZURE_ACCESS_TOKEN=azure-live OPENAI_AUTHORIZATION: Bearer provider-secret safe-context";
    let redacted = crate::application::redact_diagnostic(diagnostic);

    for secret in [
        "sk-openai-live",
        "anthropic-live",
        "azure-live",
        "provider-secret",
    ] {
        assert!(
            !redacted.contains(secret),
            "secret {secret:?} leaked in {redacted:?}"
        );
    }
    assert!(redacted.contains("safe-context"));
}
#[test]
fn malformed_settings_return_safe_diagnostic_error() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    fs::write(&settings_path, br#"{"apiKey":"secret"}"#).unwrap();

    let error = match AppService::new(settings_path) {
        Ok(_) => panic!("malformed settings must be rejected"),
        Err(error) => error,
    };

    assert_eq!(error.code, "internal-error");
    assert_eq!(error.message, "界面设置文件无法解析");
    assert!(!error.to_string().contains("secret"));
}

#[test]
fn settings_seam_persists_only_approved_lightweight_state() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    let service = AppService::new(settings_path.clone()).unwrap();
    let expected = AppSettings {
        omp_executable_path: None,
        theme: Theme::Dark,
        selected_provider_id: Some("dnslin".to_owned()),
        selected_model_id: Some("gpt-5.6-sol".to_owned()),
        cost_notice_accepted: true,
    };

    assert_eq!(
        service
            .save_ui_settings(UiSettingsUpdate {
                theme: Theme::Dark,
                selected_provider_id: Some("dnslin".to_owned()),
                selected_model_id: Some("gpt-5.6-sol".to_owned()),
                cost_notice_accepted: true,
            })
            .unwrap(),
        expected
    );

    let rebuilt = AppService::new(settings_path).unwrap();
    assert_eq!(rebuilt.get_ui_settings().unwrap(), expected);
    let persisted = fs::read_to_string(app_data.path().join("settings.json")).unwrap();
    assert!(!persisted.contains("apiKey"));
    assert!(!persisted.contains("modelRoles"));
}

#[test]
fn ui_settings_update_cannot_replace_the_confirmed_omp_executable() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/saved-omp"));

    service
        .save_ui_settings(UiSettingsUpdate {
            theme: Theme::Dark,
            selected_provider_id: Some("dnslin".to_owned()),
            selected_model_id: Some("gpt-5.6-sol".to_owned()),
            cost_notice_accepted: true,
        })
        .unwrap();

    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/saved-omp")
    );
}

#[test]
fn ui_settings_update_rejects_omp_executable_path_from_ipc() {
    let result = serde_json::from_value::<UiSettingsUpdate>(serde_json::json!({
        "ompExecutablePath": "/bin/unvalidated-omp",
        "theme": "system",
        "selectedProviderId": null,
        "selectedModelId": null,
        "costNoticeAccepted": false
    }));

    assert!(result.is_err());
}

#[test]
fn execution_io_failures_keep_a_safe_diagnostic_cause() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, None);

    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/missing-omp")),
        StartupState::InvalidExecutable { ref diagnostic_code, .. }
            if diagnostic_code == "io-not-found"
    ));
}

#[test]
fn target_inspection_failure_does_not_report_the_previous_command_exit_code() {
    let environment = Arc::new(FakeOmpEnvironment {
        inspect_target_error: Some(std::io::ErrorKind::PermissionDenied),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);

    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
        StartupState::ConfigPathFailed {
            ref diagnostic_code,
            exit_code: None,
            ..
        } if diagnostic_code == "io-permission-denied"
    ));
}
