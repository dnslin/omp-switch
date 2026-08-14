use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
};

use crate::{
    application::{
        AppService, AppSettings, OverviewLoadDto, StartupState, Theme, UiSettingsUpdate,
    },
    omp_environment::{CommandOutput, OmpEnvironment},
    target_configuration::{
        ConfigurationFileDiscovery, ConfigurationFileStatus, InitializationFailurePoint,
        TargetConfigurationDiscovery, TargetConfigurationStatus, TargetInitializationError,
        TargetInitializationExpectation, discover_target_configuration,
        initialize_target_configuration_with_failure,
    },
};
use parking_lot::Mutex;
use pretty_assertions::assert_eq;
use sha2::Digest;
use tempfile::tempdir;

#[derive(Default)]
struct FakeOmpEnvironment {
    path_omp: Option<PathBuf>,
    path_error: Option<std::io::ErrorKind>,
    calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    inspect_target_error: Option<std::io::ErrorKind>,
    config_path: Option<PathBuf>,
    inspect_real_target: bool,
    transaction_root: PathBuf,
    initialization_failure: Option<InitializationFailurePoint>,
    block_first_version: Option<(Arc<Barrier>, Arc<AtomicBool>)>,
    vary_temp_version: bool,
    temp_version_calls: AtomicUsize,
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
    fn transaction_root(&self) -> &Path {
        &self.transaction_root
    }

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
        if arguments == ["--version"]
            && let Some((barrier, used)) = &self.block_first_version
            && !used.swap(true, Ordering::AcqRel)
        {
            barrier.wait();
        }
        let name = executable.file_name().unwrap().to_string_lossy();
        match (name.as_ref(), arguments) {
            ("saved-omp", ["--version"]) => Ok(CommandOutput::success("17.4.1\n")),
            ("saved-omp", ["config", "path"]) => Ok(CommandOutput::success("/tmp/saved-agent\n")),
            ("temp-omp", ["--version"]) if self.vary_temp_version => {
                let sequence = self.temp_version_calls.fetch_add(1, Ordering::AcqRel);
                Ok(CommandOutput::success(format!("17.2.{}\n", 15 + sequence)))
            }
            ("temp-omp", ["--version"]) => Ok(CommandOutput::success("17.2.15\n")),
            ("temp-omp", ["config", "path"]) => Ok(CommandOutput::success(format!(
                "{}\n",
                self.config_path.as_ref().unwrap().display()
            ))),
            ("unknown-omp", ["--version"]) => Ok(CommandOutput::success("99.0.0\n")),
            ("unknown-omp", ["config", "path"]) => Ok(CommandOutput::success(format!(
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
        expectation: &TargetInitializationExpectation,
    ) -> Result<TargetConfigurationDiscovery, TargetInitializationError> {
        match self.initialization_failure {
            Some(failure) => {
                initialize_target_configuration_with_failure(target, expectation, failure)
            }
            None => {
                crate::target_configuration::initialize_target_configuration(target, expectation)
            }
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
        recovery_notice: None,
        create_paths: Vec::new(),
        discovery_token: format!("test:{}", target.display()),
        warnings: Vec::new(),
        issue: None,
    }
}

fn initialization_expectation(target: &Path) -> TargetInitializationExpectation {
    let discovery = discover_target_configuration(target).unwrap();
    TargetInitializationExpectation {
        create_paths: discovery.create_paths,
        discovery_token: discovery.discovery_token,
    }
}
fn service_for_target(target: &Path) -> AppService {
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.to_path_buf()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
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
fn overview_load_detects_omp_once_and_returns_shell_metadata() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment.clone(), None);

    let load = service.get_overview_load();

    assert!(load.error.is_none());
    assert!(load.overview.is_some());
    assert!(matches!(load.startup_state, StartupState::OmpReady { .. }));
    assert_eq!(
        environment.calls(),
        vec![
            (PathBuf::from("/bin/temp-omp"), vec!["--version".to_owned()]),
            (
                PathBuf::from("/bin/temp-omp"),
                vec!["config".to_owned(), "path".to_owned()]
            ),
        ]
    );
}
#[test]
fn concurrent_overview_loads_share_one_detection_and_snapshot_update() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let release_first_version = Arc::new(Barrier::new(2));
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        block_first_version: Some((
            release_first_version.clone(),
            Arc::new(AtomicBool::new(false)),
        )),
        ..FakeOmpEnvironment::default()
    });
    let service = Arc::new(service_with(environment.clone(), None));

    let first_service = service.clone();
    let first = thread::spawn(move || first_service.get_overview_load());
    assert!((0..10_000).any(|_| {
        if environment.calls().len() == 1 {
            true
        } else {
            thread::yield_now();
            false
        }
    }));

    let second_service = service.clone();
    let second = thread::spawn(move || second_service.get_overview_load());
    assert!((0..10_000).any(|_| {
        if service.overview_waiters_for_test() == 1 {
            true
        } else {
            thread::yield_now();
            false
        }
    }));
    release_first_version.wait();

    let first = first.join().unwrap();
    let second = second.join().unwrap();
    assert!(first.error.is_none());
    assert!(first.overview.is_some());
    assert!(second.error.is_none());

    assert!(second.overview.is_some());
    assert_eq!(environment.calls().len(), 2);
}
#[test]
fn overview_coalescing_preserves_joined_generation_when_a_new_load_starts() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let release_first_version = Arc::new(Barrier::new(2));
    let waiter_reached = Arc::new(Barrier::new(2));
    let release_waiter = Arc::new(Barrier::new(2));
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        block_first_version: Some((
            release_first_version.clone(),
            Arc::new(AtomicBool::new(false)),
        )),
        vary_temp_version: true,
        ..FakeOmpEnvironment::default()
    });
    let service = Arc::new(service_with(environment.clone(), None));
    service.pause_next_overview_waiter_for_test(waiter_reached.clone(), release_waiter.clone());

    let first_service = service.clone();
    let first = thread::spawn(move || first_service.get_overview_load());
    assert!((0..10_000).any(|_| {
        if environment.calls().len() == 1 {
            true
        } else {
            thread::yield_now();
            false
        }
    }));

    let second_service = service.clone();
    let second = thread::spawn(move || second_service.get_overview_load());
    assert!((0..10_000).any(|_| {
        if service.overview_waiters_for_test() == 1 {
            true
        } else {
            thread::yield_now();
            false
        }
    }));
    release_first_version.wait();
    waiter_reached.wait();

    let third_service = service.clone();
    let third = thread::spawn(move || third_service.get_overview_load());
    assert!((0..10_000).any(|_| {
        if environment.calls().len() == 4 {
            true
        } else {
            thread::yield_now();
            false
        }
    }));
    let third = third.join().unwrap();
    release_waiter.wait();
    let first = first.join().unwrap();
    let second = second.join().unwrap();

    let version = |load: &OverviewLoadDto| match &load.startup_state {
        StartupState::OmpReady { version, .. } => version.clone(),
        state => panic!("unexpected startup state: {state:?}"),
    };
    assert_eq!(version(&first), "17.2.15");
    assert_eq!(version(&second), "17.2.15");
    assert_eq!(version(&third), "17.2.16");
    assert_eq!(environment.calls().len(), 4);
}
#[test]
fn unknown_catalog_with_empty_configuration_is_read_only() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/unknown-omp")),
        config_path: Some(target),
        inspect_real_target: true,
        transaction_root: app_data.join(".app-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);

    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["state"], "read-only");
    assert_eq!(dto["counts"]["providerCount"], 0);
    assert!(
        dto["readOnlyReason"]
            .as_str()
            .unwrap()
            .contains("没有匹配的 bundled Provider 清单")
    );
    assert_eq!(dto["emptyReason"], serde_json::Value::Null);
}
#[test]
fn malformed_provider_and_role_roots_are_read_only() {
    let app_data = tempdir().unwrap().keep();
    let cases = [
        ("providers-missing", "root: {}\n", "modelRoles: {}\n"),
        ("providers-sequence", "providers: []\n", "modelRoles: {}\n"),
        (
            "providers-non-string-key",
            "providers:\n  42:\n    models: {}\n",
            "modelRoles: {}\n",
        ),
        ("roles-missing", "providers: {}\n", "settings: {}\n"),
        ("roles-sequence", "providers: {}\n", "modelRoles: []\n"),
        (
            "roles-non-string-key",
            "providers: {}\n",
            "modelRoles:\n  42: dnslin/model\n",
        ),
    ];

    for (name, models, config) in cases {
        let target = app_data.join(name);
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("models.yml"), models).unwrap();
        fs::write(target.join("config.yml"), config).unwrap();

        let dto = serde_json::to_value(
            service_for_target(&target)
                .get_overview_load()
                .overview
                .unwrap(),
        )
        .unwrap();
        assert_eq!(dto["state"], "read-only", "{name}");
        assert!(
            dto["readOnlyReason"].as_str().unwrap().contains("业务结构"),
            "{name}"
        );
    }
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
    let load = service.get_overview_load();
    assert!(matches!(
        load.startup_state,
        StartupState::OmpReady {
            requires_confirmation: true,
            ..
        }
    ));
    assert_eq!(
        load.error.as_ref().unwrap().code,
        "overview-confirmation-required"
    );
    assert!(load.overview.is_none());
    assert!(service.configuration_snapshot_for_test().is_none());

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
        .initialize_target_configuration(
            PathBuf::from("/bin/temp-omp"),
            initialization_expectation(&target),
        )
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
    let expectation = match service.detect_omp() {
        StartupState::OmpReady {
            target_configuration,
            ..
        } => TargetInitializationExpectation {
            create_paths: target_configuration.create_paths.clone(),
            discovery_token: target_configuration.discovery_token.clone(),
        },
        state => panic!("expected creation-required state, got {state:?}"),
    };
    assert_eq!(
        expectation.create_paths,
        vec![target.join("config.yml").to_string_lossy().into_owned()]
    );
    fs::remove_file(target.join("models.yml")).unwrap();

    let error = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), expectation)
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-changed");
    assert!(!target.join("models.yml").exists());
    assert!(!target.join("config.yml").exists());
}

#[test]
fn application_service_confirms_new_omp_during_target_initialization() {
    let root = tempdir().unwrap();
    let target = root.path().join("new-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
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
        .initialize_target_configuration(
            PathBuf::from("/bin/temp-omp"),
            initialization_expectation(&target),
        )
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
fn application_service_saves_first_manual_omp_during_target_initialization() {
    let root = tempdir().unwrap();
    let target = root.path().join("manual-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
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
        .initialize_target_configuration(executable.clone(), initialization_expectation(&target))
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
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        initialization_failure: Some(InitializationFailurePoint::AfterFirstCommit),
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

    let error = service
        .initialize_target_configuration(
            PathBuf::from("/bin/temp-omp"),
            initialization_expectation(&target),
        )
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-failed");
    assert!(!target.exists());
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
fn application_service_keeps_new_omp_for_incomplete_crash_recovery() {
    let root = tempdir().unwrap();
    let target = root.path().join("crashed-agent");
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        initialization_failure: Some(InitializationFailurePoint::CrashAfterFirstCommit),
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

    let error = service
        .initialize_target_configuration(
            PathBuf::from("/bin/temp-omp"),
            initialization_expectation(&target),
        )
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-failed");
    assert_eq!(
        service
            .get_ui_settings()
            .unwrap()
            .omp_executable_path
            .as_deref(),
        Some("/bin/temp-omp")
    );
    assert!(target.join("models.yml").exists());
    assert!(!target.join("config.yml").exists());

    let recovered = service.get_startup_state();

    assert!(matches!(
        recovered,
        StartupState::OmpReady {
            target_configuration,
            requires_confirmation: false,
            ..
        } if target_configuration.status == TargetConfigurationStatus::CreationRequired
            && target_configuration.recovery_notice.is_some()
    ));
    assert!(!target.join("models.yml").exists());
    assert!(matches!(
        service.get_startup_state(),
        StartupState::OmpReady {
            target_configuration,
            ..
        } if target_configuration.recovery_notice.is_some()
    ));
    assert!(matches!(
        service.detect_omp(),
        StartupState::OmpReady {
            target_configuration,
            ..
        } if target_configuration.recovery_notice.is_none()
    ));
}

#[cfg(unix)]
#[test]
fn application_service_rejects_a_retargeted_confirmation_identity() {
    use std::os::unix::fs::symlink;

    let root = tempdir().unwrap();
    let real_a = root.path().join("real-a");
    let real_b = root.path().join("real-b");
    let target = root.path().join("agent");
    fs::create_dir(&real_a).unwrap();
    fs::create_dir(&real_b).unwrap();
    symlink(&real_a, &target).unwrap();
    let service = service_for_target(&target);
    let expectation = match service.detect_omp() {
        StartupState::OmpReady {
            target_configuration,
            ..
        } => TargetInitializationExpectation {
            create_paths: target_configuration.create_paths.clone(),
            discovery_token: target_configuration.discovery_token.clone(),
        },
        state => panic!("expected creation-required state, got {state:?}"),
    };
    fs::remove_file(&target).unwrap();
    symlink(&real_b, &target).unwrap();

    let error = service
        .initialize_target_configuration(PathBuf::from("/bin/temp-omp"), expectation)
        .unwrap_err();

    assert_eq!(error.code, "target-initialization-changed");
    assert!(!real_a.join("models.yml").exists());
    assert!(!real_b.join("models.yml").exists());
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
#[test]
fn overview_reads_complete_trees_hashes_and_redacts_direct_api_keys() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    let models = r#"providers:
  dnslin:
    name: Local Provider
    baseUrl: https://user:user-info-secret@example.com/v1
    api: openai-responses
    apiKey: super-secret-api-key
    providerUnknown:
      nested: preserve-me
    models:
      gpt-5.6-sol:
        name: Sol
        api: openai-responses
        reasoning: true
        input: [text, image]
        contextWindow: 356000
        maxTokens: 32768
        modelUnknown:
          nested: preserve-model
      "gpt-5.6-sol:ultra":
        name: Ultra Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      "gpt-5.6-sol:turbo":
        name: Turbo Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      "gpt-5.6-sol/high/extra":
        name: Slash Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      incomplete:
        name: Incomplete
        api: openai-responses
        input: [text]
  other:
    baseUrl: https://example.com/v1?key=query-secret&region=us
    models:
      mystery:
        name: Mystery
  special:
    baseUrl: https:user:no-slashes-secret@example.com/v1
    models: {}
unrecognizedRoot:
  nested: untouched
"#;
    let config = r#"modelRoles:
  default: dnslin/gpt-5.6-sol:max
  advisor: dnslin/gpt-5.6-sol
  maxConfigured: dnslin/gpt-5.6-sol:max
  ultra: dnslin/gpt-5.6-sol:ultra
  turboModel: dnslin/gpt-5.6-sol:turbo
  extraSlash: dnslin/gpt-5.6-sol/high/extra
  unknown: dnslin/gpt-5.6-sol:unknown
  missingModel: dnslin/does-not-exist
  missingProvider: absent/model
  incompleteRole: dnslin/incomplete
otherSettings:
  nested:
    value: untouched
"#;
    fs::write(target.join("models.yml"), models).unwrap();
    fs::write(target.join("config.yml"), config).unwrap();

    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let dto = serde_json::to_value(overview).unwrap();

    assert_eq!(dto["state"], "normal");
    assert_eq!(dto["counts"]["providerCount"], 1);
    assert_eq!(dto["counts"]["modelCount"], 6);
    assert_eq!(dto["counts"]["roleCount"], 10);
    assert_eq!(dto["providers"][0]["hasApiKey"], true);
    assert!(!dto.to_string().contains("super-secret-api-key"));
    assert!(!dto.to_string().contains("user-info-secret"));
    assert!(!dto.to_string().contains("query-secret"));
    assert!(!dto.to_string().contains("no-slashes-secret"));
    assert!(!dto.to_string().contains("preserve-me"));
    assert!(!dto.to_string().contains("preserve-model"));
    let providers = dto["providers"].as_array().unwrap();
    let provider = |id: &str| {
        providers
            .iter()
            .find(|provider| provider["id"] == id)
            .unwrap()
    };
    assert_eq!(provider("dnslin")["classification"], "advanced");
    assert_eq!(provider("other")["classification"], "custom");
    assert_eq!(provider("special")["classification"], "unsupported");
    assert_eq!(provider("dnslin")["baseUrl"], "https://example.com/v1");
    assert_eq!(
        provider("other")["baseUrl"],
        "https://example.com/v1?region=us"
    );
    assert_eq!(
        provider("special")["baseUrl"],
        "[配置地址因无法解析而已脱敏]"
    );
    let roles = dto["roles"].as_array().unwrap();
    let role = |id: &str| roles.iter().find(|role| role["id"] == id).unwrap();
    for id in [
        "default",
        "advisor",
        "maxConfigured",
        "ultra",
        "extraSlash",
        "turboModel",
    ] {
        assert_eq!(role(id)["status"], "configured");
        assert!(role(id)["selector"].is_string());
    }
    assert_eq!(role("unknown")["status"], "advanced");
    assert_eq!(role("unknown")["selector"], serde_json::Value::Null);
    assert_eq!(role("missingModel")["status"], "model-missing");
    assert_eq!(role("missingProvider")["status"], "provider-missing");
    assert_eq!(role("incompleteRole")["status"], "incomplete");
    assert_eq!(role("ultra")["selector"], "dnslin/gpt-5.6-sol:ultra");
    assert_eq!(role("turboModel")["selector"], "dnslin/gpt-5.6-sol:turbo");
    assert_eq!(
        role("extraSlash")["selector"],
        "dnslin/gpt-5.6-sol/high/extra"
    );

    let snapshot = service.configuration_snapshot_for_test().unwrap();
    assert_eq!(
        snapshot.models.tree["unrecognizedRoot"]["nested"],
        "untouched"
    );
    assert_eq!(
        snapshot.models.tree["providers"]["dnslin"]["providerUnknown"]["nested"],
        "preserve-me"
    );
    assert_eq!(
        snapshot.config.tree["otherSettings"]["nested"]["value"],
        "untouched"
    );
    assert_eq!(
        snapshot.models.raw_hash,
        sha2::Sha256::digest(models.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    );
}

#[test]
fn overview_counts_only_custom_providers_and_marks_overrides() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  openai:
    models:
      gpt-5.6-sol:
        name: Bundled
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  missing:
    models:
      local:
        name: Missing URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  malformed:
    baseUrl: ftp://example.com
    models:
      local:
        name: Malformed URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  nonString:
    baseUrl: 42
    models:
      local:
        name: Non-string URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  empty:
    models: {}
  custom:
    baseUrl: https://example.com
    models:
      local:
        name: Local
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/local\n  alias: '@default'\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    assert_eq!(dto["state"], "normal");
    assert_eq!(dto["counts"]["providerCount"], 1);
    assert_eq!(dto["counts"]["modelCount"], 5);
    assert_eq!(dto["counts"]["roleCount"], 2);
    let providers = dto["providers"].as_array().unwrap();
    let provider = |id: &str| {
        providers
            .iter()
            .find(|provider| provider["id"] == id)
            .unwrap()
    };
    assert_eq!(provider("openai")["classification"], "built-in-override");
    assert_eq!(provider("empty")["classification"], "unsupported");
    assert_eq!(provider("custom")["classification"], "custom");
    for id in ["missing", "malformed", "nonString"] {
        let provider = provider(id);
        assert_eq!(provider["classification"], "unsupported");
        assert_eq!(provider["editable"], false);
        assert!(
            provider["readOnlyReason"]
                .as_str()
                .unwrap()
                .contains("HTTP(S) Base URL")
        );
    }
    let roles = dto["roles"].as_array().unwrap();
    let role = |id: &str| roles.iter().find(|role| role["id"] == id).unwrap();
    assert_eq!(role("default")["status"], "configured");
    assert_eq!(role("alias")["status"], "advanced");
}

#[test]
fn overview_parse_failure_clears_previous_business_snapshot() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

    let service = service_for_target(&target);
    let _ = service.get_overview_load();
    fs::write(target.join("models.yml"), "providers: [\n").unwrap();

    let error = service.get_overview_load().error.unwrap();
    assert_eq!(error.code, "overview-parse-error");
    assert!(service.configuration_snapshot_for_test().is_none());
}
#[test]
fn overview_reads_yaml_only_targets_as_read_only() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yaml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yaml"), "modelRoles: {}\n").unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    assert_eq!(dto["state"], "read-only");
    assert_eq!(dto["files"]["models"]["status"], "alternate-only");
    assert_eq!(dto["files"]["config"]["status"], "alternate-only");
    assert!(service.configuration_snapshot_for_test().is_some());
}

#[test]
fn overview_missing_canonical_files_returns_empty_without_business_snapshot() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    assert_eq!(dto["state"], "empty");
    assert_eq!(dto["counts"]["providerCount"], 0);
    assert_eq!(dto["counts"]["modelCount"], 0);
    assert_eq!(dto["counts"]["roleCount"], 0);
    assert!(service.configuration_snapshot_for_test().is_none());
}
