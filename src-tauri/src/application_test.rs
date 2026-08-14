use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Barrier},
    thread,
};

use crate::{
    application::{AppService, AppSettings, StartupState, Theme, UiSettingsUpdate},
    omp_environment::{CommandOutput, OmpEnvironment},
    target_configuration::{
        ConfigurationFileDiscovery, ConfigurationFileStatus, InitializationFailurePoint,
        TargetConfigurationDiscovery, TargetConfigurationStatus, discover_target_configuration,
        initialize_target_configuration_with_failure,
    },
};
use parking_lot::Mutex;
use pretty_assertions::assert_eq;
use tempfile::tempdir;

#[derive(Default)]
struct FakeOmpEnvironment {
    path_omp: Option<PathBuf>,
    path_error: Option<std::io::ErrorKind>,
    calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    inspect_target_error: Option<std::io::ErrorKind>,
    config_path: Option<PathBuf>,
    inspect_real_target: bool,
    initialization_failure: Option<InitializationFailurePoint>,
}

impl FakeOmpEnvironment {
    fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            path_omp: Some(path.into()),
            ..Self::default()
        }
    }

    fn with_path_error(kind: std::io::ErrorKind) -> Self {
        Self {
            path_error: Some(kind),
            ..Self::default()
        }
    }

    fn calls(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.calls.lock().clone()
    }
}

impl OmpEnvironment for FakeOmpEnvironment {
    fn find_in_path(&self) -> std::io::Result<Option<PathBuf>> {
        if let Some(kind) = self.path_error {
            return Err(std::io::Error::new(kind, "PATH discovery failed"));
        }
        Ok(self.path_omp.clone())
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
            ("temp-omp", ["--version"]) => Ok(CommandOutput::success("18.1.0\n")),
            ("temp-omp", ["config", "path"]) => Ok(CommandOutput::success(format!(
                "{}\n",
                self.config_path.as_ref().unwrap().display()
            ))),
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

    fn inspect_target(&self, target: &Path) -> std::io::Result<TargetConfigurationDiscovery> {
        if let Some(kind) = self.inspect_target_error {
            return Err(std::io::Error::new(kind, "target inspection failed"));
        }
        if self.inspect_real_target {
            return discover_target_configuration(target);
        }
        Ok(writable_target(target))
    }

    fn initialize_target(
        &self,
        target: &Path,
        expected_create_paths: &[String],
    ) -> std::io::Result<TargetConfigurationDiscovery> {
        match self.initialization_failure {
            Some(failure) => {
                initialize_target_configuration_with_failure(target, expected_create_paths, failure)
            }
            None => crate::target_configuration::initialize_target_configuration(
                target,
                expected_create_paths,
            ),
        }
    }
}

fn writable_target(target: &Path) -> TargetConfigurationDiscovery {
    let file = |name: &str| ConfigurationFileDiscovery {
        canonical_path: target.join(name).to_string_lossy().into_owned(),
        resolved_path: Some(target.join(name).to_string_lossy().into_owned()),
        status: ConfigurationFileStatus::Normal,
    };
    TargetConfigurationDiscovery {
        path: target.to_string_lossy().into_owned(),
        resolved_path: Some(target.to_string_lossy().into_owned()),
        status: TargetConfigurationStatus::Writable,
        writable: true,
        models: file("models.yml"),
        config: file("config.yml"),
        create_paths: Vec::new(),
        warnings: Vec::new(),
        issue: None,
    }
}

fn creation_paths(target: &Path) -> Vec<String> {
    discover_target_configuration(target).unwrap().create_paths
}
fn service_for_target(target: &Path) -> AppService {
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.to_path_buf()),
        inspect_real_target: true,
        ..FakeOmpEnvironment::default()
    });
    service_with(environment, None)
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
            target_configuration: Box::new(writable_target(Path::new("/tmp/saved-agent"))),
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
fn startup_detection_preserves_saved_executable_failure_when_path_is_empty() {
    let environment = Arc::new(FakeOmpEnvironment::default());
    let service = service_with(environment, Some("/bin/broken-version"));

    assert!(matches!(
        service.detect_omp(),
        StartupState::VersionFailed {
            exit_code: Some(7),
            ref stderr,
            ..
        } if stderr.contains("已脱敏") && !stderr.contains("super-secret")
    ));
}

#[test]
fn startup_detection_reports_path_discovery_failure_when_no_saved_candidate_exists() {
    let environment = Arc::new(FakeOmpEnvironment::with_path_error(
        std::io::ErrorKind::InvalidInput,
    ));
    let service = service_with(environment, None);

    assert!(matches!(
        service.detect_omp(),
        StartupState::OmpUnavailable { ref message }
            if message.contains("无法检查系统 PATH") && message.contains("io-invalid-input")
    ));
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
        } if target_configuration.path == "/tmp/path-agent"
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
fn failed_confirmation_persistence_can_retry_without_revalidation() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&AppSettings {
            omp_executable_path: Some("/bin/saved-omp".to_owned()),
            ..AppSettings::default()
        })
        .unwrap(),
    )
    .unwrap();
    let service = AppService::new_with_environment(
        settings_path.clone(),
        Arc::new(FakeOmpEnvironment::default()),
    )
    .unwrap();
    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
        StartupState::OmpReady { .. }
    ));

    fs::remove_file(&settings_path).unwrap();
    fs::create_dir(&settings_path).unwrap();
    assert!(
        service
            .confirm_selected_omp(PathBuf::from("/bin/path-omp"))
            .is_err()
    );
    fs::remove_dir(&settings_path).unwrap();

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
    let redacted = crate::redaction::redact_diagnostic(diagnostic);
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
fn diagnostics_redact_compact_authorization_and_standalone_separators() {
    for diagnostic in [
        "Authorization:Bearer abc.def safe-context",
        "API_KEY = secret safe-context",
        "API_KEY : colon-secret safe-context",
        "OPENAI_API_KEY = provider-secret safe-context",
    ] {
        let redacted = crate::redaction::redact_diagnostic(diagnostic);
        for secret in ["abc.def", "secret", "colon-secret", "provider-secret"] {
            assert!(
                !redacted.contains(secret),
                "secret {secret:?} leaked in {redacted:?} for {diagnostic:?}"
            );
        }
        assert!(redacted.contains("safe-context"));
    }
}

#[test]
fn diagnostics_suppress_structured_json_and_url_credentials() {
    for diagnostic in [
        r#"request failed: {\"token\":\"json-secret\",\"message\":\"denied\"}"#,
        "request failed: https://example.test/models?api_key=query-secret&limit=10",
    ] {
        let redacted = crate::redaction::redact_diagnostic(diagnostic);
        assert_eq!(redacted, "[诊断信息因可能包含凭据而已脱敏]");
        assert!(!redacted.contains("json-secret"));
        assert!(!redacted.contains("query-secret"));
    }
}

#[test]
fn diagnostics_redact_provider_prefixed_secret_names() {
    let diagnostic = "OPENAI_API_KEY=sk-openai-live ANTHROPIC_API_KEY anthropic-live AZURE_ACCESS_TOKEN=azure-live OPENAI_AUTHORIZATION: Bearer provider-secret safe-context";
    let redacted = crate::redaction::redact_diagnostic(diagnostic);

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
fn settings_update_atomically_replaces_an_existing_settings_file() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&AppSettings {
            theme: Theme::Light,
            ..AppSettings::default()
        })
        .unwrap(),
    )
    .unwrap();
    let service = AppService::new(settings_path.clone()).unwrap();

    service
        .save_ui_settings(UiSettingsUpdate {
            theme: Theme::Dark,
            selected_provider_id: None,
            selected_model_id: None,
            cost_notice_accepted: false,
        })
        .unwrap();

    let persisted: AppSettings = serde_json::from_slice(&fs::read(settings_path).unwrap()).unwrap();
    assert_eq!(persisted.theme, Theme::Dark);
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
fn concurrent_settings_update_and_omp_confirmation_preserve_both_changes() {
    for _ in 0..100 {
        let environment = Arc::new(FakeOmpEnvironment::default());
        let service = Arc::new(service_with(environment, Some("/bin/saved-omp")));
        assert!(matches!(
            service.validate_selected_omp(PathBuf::from("/bin/path-omp")),
            StartupState::OmpReady { .. }
        ));

        let barrier = Arc::new(Barrier::new(3));
        let settings_service = Arc::clone(&service);
        let settings_barrier = Arc::clone(&barrier);
        let settings_thread = thread::spawn(move || {
            settings_barrier.wait();
            settings_service.save_ui_settings(UiSettingsUpdate {
                theme: Theme::Dark,
                selected_provider_id: Some("dnslin".to_owned()),
                selected_model_id: None,
                cost_notice_accepted: false,
            })
        });
        let confirmation_service = Arc::clone(&service);
        let confirmation_barrier = Arc::clone(&barrier);
        let confirmation_thread = thread::spawn(move || {
            confirmation_barrier.wait();
            confirmation_service.confirm_selected_omp(PathBuf::from("/bin/path-omp"))
        });

        barrier.wait();
        assert!(settings_thread.join().unwrap().is_ok());
        assert!(confirmation_thread.join().unwrap().is_ok());
        let settings = service.get_ui_settings().unwrap();
        assert_eq!(settings.theme, Theme::Dark);
        assert_eq!(
            settings.omp_executable_path.as_deref(),
            Some("/bin/path-omp")
        );
    }
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
fn application_service_discovers_legacy_json_and_yaml_parse_failures() {
    let root = tempdir().unwrap();
    let legacy = root.path().join("legacy-agent");
    fs::create_dir(&legacy).unwrap();
    fs::write(legacy.join("models.json"), "{\"providers\":{}}\n").unwrap();
    fs::write(legacy.join("settings.json"), "{\"modelRoles\":{}}\n").unwrap();
    let legacy_state = service_for_target(&legacy).detect_omp();
    assert!(matches!(
        legacy_state,
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::MigrationRequired
    ));

    let malformed = root.path().join("malformed-agent");
    fs::create_dir(&malformed).unwrap();
    fs::write(malformed.join("models.yml"), "providers: [\n").unwrap();
    fs::write(malformed.join("config.yml"), "modelRoles: {}\n").unwrap();
    let malformed_state = service_for_target(&malformed).detect_omp();
    assert!(matches!(
        malformed_state,
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::ParseError
                && target_configuration.issue.is_some()
    ));
}

#[test]
fn application_service_classifies_yaml_extension_combinations() {
    let root = tempdir().unwrap();
    let alternate_only = root.path().join("alternate-agent");
    fs::create_dir(&alternate_only).unwrap();
    fs::write(alternate_only.join("models.yaml"), "providers: {}\n").unwrap();
    fs::write(alternate_only.join("config.yaml"), "modelRoles: {}\n").unwrap();
    assert!(matches!(
        service_for_target(&alternate_only).detect_omp(),
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::ReadOnly
                && target_configuration.models.status == ConfigurationFileStatus::AlternateOnly
                && target_configuration.config.status == ConfigurationFileStatus::AlternateOnly
    ));

    let canonical_with_alternate = root.path().join("mixed-agent");
    fs::create_dir(&canonical_with_alternate).unwrap();
    fs::write(
        canonical_with_alternate.join("models.yml"),
        "providers: {}\n",
    )
    .unwrap();
    fs::write(
        canonical_with_alternate.join("models.yaml"),
        "providers: {}\n",
    )
    .unwrap();
    fs::write(
        canonical_with_alternate.join("config.yml"),
        "modelRoles: {}\n",
    )
    .unwrap();
    assert!(matches!(
        service_for_target(&canonical_with_alternate).detect_omp(),
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::Writable
                && target_configuration.models.status == ConfigurationFileStatus::CanonicalWithAlternate
    ));
}

#[test]
fn application_service_initializes_missing_target_and_returns_reparsed_state() {
    let root = tempdir().unwrap();
    let target = root.path().join("agent");
    let service = service_for_target(&target);

    assert!(matches!(
        service.detect_omp(),
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::CreationRequired
    ));
    let state = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), creation_paths(&target))
        .unwrap();

    assert!(matches!(
        state,
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.status == TargetConfigurationStatus::Writable
    ));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        b"providers: {}\n"
    );
    assert_eq!(
        fs::read(target.join("config.yml")).unwrap(),
        b"modelRoles: {}\n"
    );
}

#[test]
fn application_service_rejects_a_changed_confirmed_creation_list() {
    let root = tempdir().unwrap();
    let target = root.path().join("changed-agent");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    let service = service_for_target(&target);
    let expected_create_paths = match service.detect_omp() {
        StartupState::OmpReady {
            target_configuration,
            ..
        } => target_configuration.create_paths.clone(),
        state => panic!("expected creation-required state, got {state:?}"),
    };
    assert_eq!(
        expected_create_paths,
        vec![target.join("config.yml").to_string_lossy().into_owned()]
    );
    fs::remove_file(target.join("models.yml")).unwrap();

    let error = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), expected_create_paths)
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-changed");
    assert!(!target.join("models.yml").exists());
    assert!(!target.join("config.yml").exists());
}

#[test]
fn application_service_confirms_new_omp_before_initializing_its_target() {
    let root = tempdir().unwrap();
    let target = root.path().join("new-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, Some("/bin/saved-omp"));
    assert!(matches!(
        service.validate_selected_omp(PathBuf::from("/bin/temp-omp")),
        StartupState::OmpReady {
            requires_confirmation: true,
            ..
        }
    ));

    let state = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), creation_paths(&target))
        .unwrap();

    assert!(matches!(
        state,
        StartupState::OmpReady {
            requires_confirmation: false,
            ..
        }
    ));
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/temp-omp")
    );
}

#[test]
fn application_service_saves_first_manual_omp_before_creating_its_target() {
    let root = tempdir().unwrap();
    let target = root.path().join("manual-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        config_path: Some(target.clone()),
        inspect_real_target: true,
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);
    let executable = PathBuf::from("/bin/temp-omp");

    assert!(matches!(
        service.validate_selected_omp(executable.clone()),
        StartupState::OmpReady {
            requires_confirmation: true,
            ..
        }
    ));
    let state = service
        .initialize_target_configuration(executable.clone(), creation_paths(&target))
        .unwrap();

    assert!(matches!(
        state,
        StartupState::OmpReady {
            requires_confirmation: false,
            ..
        }
    ));
    assert_eq!(
        service.get_ui_settings().unwrap().omp_executable_path,
        Some(executable.to_string_lossy().into_owned())
    );
    assert!(service.confirm_selected_omp(executable).is_err());
}

#[test]
fn application_service_rolls_back_partial_target_initialization() {
    let root = tempdir().unwrap();
    let target = root.path().join("partial-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        initialization_failure: Some(InitializationFailurePoint::AfterFirstCommit),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);

    let error = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), creation_paths(&target))
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-failed");
    assert!(!target.exists());
}
#[cfg(unix)]
#[test]
fn application_service_reports_symlink_real_target() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let real = root.path().join("real-agent");
    let linked = root.path().join("linked-agent");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(real.join("config.yml"), "modelRoles: {}\n").unwrap();
    symlink(&real, &linked).unwrap();

    let state = service_for_target(&linked).detect_omp();

    assert!(matches!(
        state,
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.resolved_path
                == Some(real.canonicalize().unwrap().to_string_lossy().into_owned())
    ));
}

#[cfg(windows)]
#[test]
fn application_service_reports_junction_real_target() {
    use std::process::Command;

    let root = tempdir().unwrap();
    let real = root.path().join("real-agent");
    let junction = root.path().join("junction-agent");
    fs::create_dir(&real).unwrap();
    fs::write(real.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(real.join("config.yml"), "modelRoles: {}\n").unwrap();
    let command = format!(
        "mklink /J \"{}\" \"{}\"",
        junction.display(),
        real.display()
    );
    assert!(
        Command::new("cmd")
            .args(["/C", &command])
            .status()
            .unwrap()
            .success()
    );

    let state = service_for_target(&junction).detect_omp();

    assert!(matches!(
        state,
        StartupState::OmpReady { target_configuration, .. }
            if target_configuration.resolved_path
                == Some(real.canonicalize().unwrap().to_string_lossy().into_owned())
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
