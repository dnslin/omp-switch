#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Barrier,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver},
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

#[cfg(unix)]
use std::ffi::CString;

use crate::{
    application::{
        AppService, AppSettings, CreateCustomProviderInput, CreateModelFields, CreateModelInput,
        CreateProviderFields, DeleteModelInput, DeleteProviderInput, DirectApiKeyIntent,
        EditCustomProviderInput, EditModelInput, ModelDefinitionFields, ModelEditFields,
        ModelsWriteFailurePoint, OverviewLoadDto, ProviderAuthMode, StartupState, SupportedApi,
        SupportedInput, Theme, UiSettingsUpdate,
    },
    omp_environment::{CommandOutput, CommandRunError, OmpEnvironment, SystemOmpEnvironment},
    overview::{ModelTestConfiguration, OverviewAuthMode},
    target_configuration::{
        ConfigurationFileDiscovery, ConfigurationFileStatus, ConfigurationIssue,
        InitializationFailurePoint, TargetConfigurationDiscovery, TargetConfigurationStatus,
        TargetInitializationError, TargetInitializationExpectation, discover_target_configuration,
        initialize_target_configuration_with_failure,
    },
};
use fs4::FileExt;
use parking_lot::Mutex;
use pretty_assertions::assert_eq;
use sha2::{Digest, Sha256};
use tempfile::tempdir;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::prelude::*;

#[derive(Clone)]
struct CleanupWarningCounter(Arc<AtomicUsize>);

struct CleanupOperationVisitor {
    is_cleanup: bool,
}

impl tracing::field::Visit for CleanupOperationVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "operation" && value == "cleanup_partial_models_backup" {
            self.is_cleanup = true;
        }
    }

    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
}

impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for CleanupWarningCounter {
    fn on_event(&self, event: &tracing::Event<'_>, _: tracing_subscriber::layer::Context<'_, S>) {
        if *event.metadata().level() != tracing::Level::WARN {
            return;
        }
        let mut visitor = CleanupOperationVisitor { is_cleanup: false };
        event.record(&mut visitor);
        if visitor.is_cleanup {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
}

#[derive(Default)]
struct FakeOmpEnvironment {
    path_omp: Option<PathBuf>,
    path_error: Option<std::io::ErrorKind>,
    calls: Mutex<Vec<(PathBuf, Vec<String>)>>,
    inspect_target_error: Option<std::io::ErrorKind>,
    target_override: Option<TargetConfigurationDiscovery>,
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
    fn run_with_deadline(
        &self,
        executable: &Path,
        arguments: &[&str],
        cancellation: &CancellationToken,
        deadline: Instant,
    ) -> Result<CommandOutput, CommandRunError> {
        if cancellation.is_cancelled() {
            return Err(CommandRunError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(CommandRunError::TimedOut);
        }
        self.run(executable, arguments).map_err(CommandRunError::Io)
    }
    fn inspect_target(&self, target: &Path) -> std::io::Result<TargetConfigurationDiscovery> {
        if let Some(kind) = self.inspect_target_error {
            return Err(std::io::Error::new(kind, "target inspection failed"));
        }
        if let Some(target_override) = &self.target_override {
            return Ok(target_override.clone());
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
fn service_for_target_with_app_data(target: &Path, app_data: &Path) -> AppService {
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.to_path_buf()),
        inspect_real_target: true,
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        ..FakeOmpEnvironment::default()
    });
    AppService::new_with_environment(app_data.join("settings.json"), environment).unwrap()
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
fn application_service_uses_an_exact_catalog_and_locks_only_provider_management_when_missing() {
    let app_data = tempdir().unwrap().keep();
    let exact_target = app_data.join("exact-agent");
    fs::create_dir_all(&exact_target).unwrap();
    fs::write(
        exact_target.join("models.yml"),
        r#"providers:
  OPENAI:
    baseUrl: https://api.openai.com/v1
    api: openai-responses
    models:
      - id: GPT-5.6-SOL
        name: Bundled override
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
  advanced:
    baseUrl: https://advanced.example/v1
    api: openai-responses
    headers:
      x-provider-mode: advanced
    models:
      - id: local
        name: Local
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
  custom:
    baseUrl: https://custom.example/v1
    api: openai-responses
    models:
      - id: too-large
        name: Too Large
        input: [text]
        contextWindow: 1000
        maxTokens: 2000

"#,
    )
    .unwrap();
    fs::write(exact_target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let exact_environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(exact_target.clone()),
        inspect_real_target: true,
        transaction_root: app_data.join("exact-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let exact_service = service_with(exact_environment.clone(), None);

    let exact = serde_json::to_value(exact_service.get_overview_load().overview.unwrap()).unwrap();
    let provider = |id: &str| {
        exact["providers"]
            .as_array()
            .unwrap()
            .iter()
            .find(|provider| provider["id"] == id)
            .unwrap()
    };
    assert_eq!(provider("OPENAI")["classification"], "built-in-override");
    assert_eq!(provider("OPENAI")["editable"], false);
    assert_eq!(provider("advanced")["classification"], "advanced");
    assert_eq!(provider("advanced")["editable"], false);
    assert_eq!(provider("custom")["editable"], true);
    assert_eq!(provider("custom")["models"][0]["complete"], false);
    assert_eq!(provider("custom")["models"][0]["status"], "incomplete");
    assert_eq!(
        exact_environment.calls(),
        vec![
            (PathBuf::from("/bin/temp-omp"), vec!["--version".to_owned()]),
            (
                PathBuf::from("/bin/temp-omp"),
                vec!["config".to_owned(), "path".to_owned()]
            ),
        ]
    );

    for (field, value) in [
        ("oauth", "oauth: true"),
        ("command credential", "apiKey: !command echo-secret"),
        ("custom header", "headers:\n      x-provider-mode: custom"),
        ("compat", "compat: openai"),
        ("discovery", "discovery: true"),
        ("model overrides", "modelOverrides: {}"),
        ("transport", "transport: fetch"),
        ("remote compaction", "remoteCompaction: true"),
        ("strict tools", "disableStrictTools: true"),
        ("auth header", "authHeader: Authorization"),
    ] {
        let target = app_data.join(format!("advanced-{field}"));
        fs::create_dir_all(&target).unwrap();
        fs::write(
            target.join("models.yml"),
            format!(
                "providers:\n  advanced:\n    baseUrl: https://advanced.example/v1\n    api: openai-responses\n    {value}\n    models:\n      - id: local\n        name: Local\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
            ),
        )
        .unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

        let dto = serde_json::to_value(
            service_for_target(&target)
                .get_overview_load()
                .overview
                .unwrap(),
        )
        .unwrap();
        assert_eq!(dto["providers"][0]["classification"], "advanced", "{field}");
        assert_eq!(dto["providers"][0]["editable"], false, "{field}");
    }

    let missing_target = app_data.join("missing-agent");
    fs::create_dir_all(&missing_target).unwrap();
    fs::write(
        missing_target.join("models.yml"),
        "providers:\n  custom:\n    baseUrl: https://custom.example/v1\n    api: openai-responses\n    models:\n      - id: local\n        name: Local\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n",
    )
    .unwrap();
    fs::write(
        missing_target.join("config.yml"),
        "modelRoles:\n  default: custom/local\n",
    )
    .unwrap();
    let missing_environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/unknown-omp")),
        config_path: Some(missing_target),
        inspect_real_target: true,
        transaction_root: app_data.join("missing-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let missing = serde_json::to_value(
        service_with(missing_environment, None)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();

    assert_eq!(missing["state"], "read-only");
    assert_eq!(missing["providers"][0]["classification"], "unavailable");
    assert_eq!(missing["providers"][0]["editable"], false);
    assert_eq!(missing["roles"][0]["status"], "configured");
    assert!(
        missing["readOnlyReason"]
            .as_str()
            .unwrap()
            .contains("没有匹配的 bundled Provider 清单")
    );
}
#[test]
fn malformed_provider_and_role_roots_are_read_only() {
    let app_data = tempdir().unwrap().keep();
    let cases = [
        ("providers-missing", "root: {}\n", "modelRoles: {}\n"),
        ("providers-sequence", "providers: []\n", "modelRoles: {}\n"),
        (
            "providers-non-string-key",
            "providers:\n  42:\n    models: []\n",
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
#[tokio::test]
async fn model_test_rejects_an_unconfirmed_path_omp_replacement() {
    let environment = Arc::new(FakeOmpEnvironment::with_path("/bin/path-omp"));
    let service = service_with(environment.clone(), Some("/bin/missing-omp"));
    service.accept_model_test_cost_notice().unwrap();

    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "provider",
        "modelId": "model"
    }))
    .unwrap();
    let error = service.test_model(input).await.unwrap_err();

    assert_eq!(error.code, "model-test-omp-confirmation-required");
    assert_eq!(
        environment.calls(),
        vec![(
            PathBuf::from("/bin/missing-omp"),
            vec!["--version".to_owned()]
        )]
    );
}

#[cfg(unix)]
#[test]
fn omp_deadline_covers_inherited_output_pipes() {
    let root = tempdir().unwrap();
    let executable = root.path().join("spawn-descendant");
    fs::write(&executable, "#!/bin/sh\nsleep 5 &\nexit 0\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let environment = SystemOmpEnvironment::new(root.path().join("transactions"));
    let started = Instant::now();
    let result = environment.run_with_deadline(
        &executable,
        &[],
        &CancellationToken::new(),
        Instant::now() + Duration::from_millis(50),
    );

    assert!(matches!(result, Err(CommandRunError::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[cfg(unix)]
#[test]
fn omp_command_output_is_bounded() {
    let root = tempdir().unwrap();
    let executable = root.path().join("noisy-omp");
    fs::write(
        &executable,
        "#!/bin/sh\nwhile :; do printf '%8192s' ''; done\n",
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let environment = SystemOmpEnvironment::new(root.path().join("transactions"));
    let result = environment.run_with_deadline(
        &executable,
        &[],
        &CancellationToken::new(),
        Instant::now() + Duration::from_secs(2),
    );

    assert!(matches!(
        result,
        Err(CommandRunError::Io(error))
            if error.kind() == std::io::ErrorKind::InvalidData
    ));
}
#[cfg(windows)]
#[test]
fn omp_deadline_covers_windows_inherited_output_pipes() {
    let environment = SystemOmpEnvironment::new(PathBuf::from("transactions"));
    let started = Instant::now();
    let result = environment.run_with_deadline(
        Path::new("cmd.exe"),
        &[
            "/D",
            "/C",
            "start \"\" /B cmd.exe /D /C \"ping.exe -n 6 127.0.0.1 >NUL\"",
        ],
        &CancellationToken::new(),
        Instant::now() + Duration::from_millis(50),
    );

    assert!(matches!(result, Err(CommandRunError::TimedOut)));
    assert!(started.elapsed() < Duration::from_secs(1));
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
fn diagnostics_redact_common_secret_assignments() {
    let diagnostic = "API_KEY super-secret password=hunter2 token 'quoted secret' client_secret: hidden access_token= spaced x-api-key: header-secret sk-live-raw safe-context";
    let redacted = crate::redaction::redact_diagnostic(diagnostic);
    for secret in [
        "super-secret",
        "hunter2",
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
fn diagnostics_fail_closed_for_ambiguous_secret_assignments() {
    for diagnostic in [
        "safe-context,api_key=punctuated-secret",
        "(token=wrapped-secret)",
        "%74oken=encoded-secret",
        "Authoriz%61tion: Bearer encoded-header-secret",
        "(token = wrapped-secret)",
        "safe-context,api_key : punctuated-secret",
        "%74oken = encoded-secret",
        "safe%2Capi_key=opaque-value",
        "token%3Dopaque-secret",
        "safe%2Capi_key%3Dopaque-value",
        "Authorization%3A%20Bearer%20abc.def",
        "%74oken %3D opaque-secret",
    ] {
        assert_eq!(
            crate::redaction::redact_diagnostic(diagnostic),
            "[诊断信息因可能包含凭据而已脱敏]"
        );
    }
}
#[test]
fn diagnostics_redact_standalone_secret_separators() {
    for diagnostic in [
        "API_KEY = secret safe-context",
        "API_KEY : colon-secret safe-context",
        "OPENAI_API_KEY = provider-secret safe-context",
    ] {
        let redacted = crate::redaction::redact_diagnostic(diagnostic);
        for secret in ["secret", "colon-secret", "provider-secret"] {
            assert!(
                !redacted.contains(secret),
                "secret {secret:?} leaked in {redacted:?} for {diagnostic:?}"
            );
        }
        assert!(redacted.contains("safe-context"));
    }
}
#[test]
fn diagnostics_fail_closed_for_unsafe_authorization_headers() {
    for diagnostic in [
        "Authorization:Bearer abc.def safe-context",
        "Authorization : Bearer abc.def safe-context",
        "OPENAI_AUTHORIZATION: Bearer provider-secret safe-context",
        "Authorization: Basic dXNlcjpwYXNz",
        "Authorization Basic dXNlcjpwYXNz",
        "Authorization opaque-token secret-value",
        "Authorization failed with Basic dXNlcjpwYXNz",
        "Authorization header AWS4-HMAC-SHA256 Credential=access-key Signature=signature-value",
        "Authorization: AWS4-HMAC-SHA256 Credential=access-key Signature=signature-value",
        "Authorization AWS4-HMAC-SHA256 Credential=access-key Signature=signature-value",
        "Authorization: Bearer abc.def Basic dXNlcjpwYXNz",
        "Authorization: Bearer abc.def AWS4-HMAC-SHA256 Credential=access-key Signature=signature-value",
        "Authorization: Bearer abc.def Credential=access-key",
        "Authorization Bearer abc.def Basic dXNlcjpwYXNz",
        "Authorization: Bearer abc.def,Basic dXNlcjpwYXNz",
        "Authorization: Bearer abc.def (token=wrapped-secret)",
        "context%3AAuthorization: Bearer abc.def",
        "context%3AProxy-Authorization: Basic cHJveHk6cGFzcw==",
        "Authorization: Bearer abc.def safe-context,api_key=punctuated-secret",
        "context:Authorization Bearer abc.def",
        "context:Proxy-Authorization Basic cHJveHk6cGFzcw==",
        "Proxy-Authorization: Basic cHJveHk6cGFzcw==",
        "Proxy-Authorization Basic cHJveHk6cGFzcw==",
    ] {
        let redacted = crate::redaction::redact_diagnostic(diagnostic);
        assert_eq!(redacted, "[诊断信息因可能包含凭据而已脱敏]");
        assert!(!redacted.contains("dXNlcjpwYXNz"));
        assert!(!redacted.contains("access-key"));
        assert!(!redacted.contains("signature-value"));
    }
}

#[test]
fn diagnostics_preserve_non_header_authorization_context() {
    let redacted = crate::redaction::redact_diagnostic("authorization failed for provider");
    assert_ne!(redacted, "[诊断信息因可能包含凭据而已脱敏]");
    assert!(redacted.contains("authorization"));
    assert!(redacted.contains("failed"));
    assert!(redacted.contains("provider"));
}

#[test]
fn diagnostics_suppress_structured_json_and_url_credentials() {
    for diagnostic in [
        r#"request failed: {\"token\":\"json-secret\",\"message\":\"denied\"}"#,
        "request failed: https://example.test/models/sk-live-abc123#fragment-xyz",
        "request failed: https://example.test/models?api_key=query-secret&limit=10",
        "request failed: https://example.test/v1/%73ecret/opaque123",
        "request failed: https://example.test/v1/%74oken%3Dopaque123",
        "request failed: https:user:password@example.com/v1",
        "request failed: endpoint=|https:alice:s3cr3t@example.com/v1|",
        "request failed: endpoint=https:user:password@example.com/v1",
        "request failed: endpoint=<https:alice:s3cr3t@example.com/v1>",
        "request failed: endpoint=`https:alice:s3cr3t@example.com/v1`",
        "request failed: (https:alice:s3cr3t@example.com/v1)",
        "request failed: endpoint=(https:alice:s3cr3t@example.com/v1)",
        "request:endpoint=https:alice:s3cr3t@example.com/v1",
    ] {
        let redacted = crate::redaction::redact_diagnostic(diagnostic);
        assert_eq!(redacted, "[诊断信息因可能包含凭据而已脱敏]");
        assert!(!redacted.contains("json-secret"));
        assert!(!redacted.contains("abc123"));
        assert!(!redacted.contains("fragment-xyz"));
    }
}

#[test]
fn projection_rejects_encoded_paths_and_unknown_query_parameters() {
    for address in [
        "https://example.test/v1/sk%2Dlive%2Dabc",
        "https://example.test/v1/api%5Fkey%3Dsecret",
        "https://example.test/v1/%74oken/opaque-credential",
        "https://example.test/v1/%73ecret/value",
        "https://example.test/v1/openai%2Dapi%2Dkey%3Dopaque123",
        "https://example.test/v1/%61pikey/value",
        "https://example.test/v1/%70asswd/value",
        "https://example.test/v1?tenant=acme",
    ] {
        assert_eq!(
            crate::redaction::redact_projection(address),
            "[配置地址因无法解析而已脱敏]",
            "unsafe address was projected: {address}"
        );
    }
}

#[test]
fn diagnostics_redact_provider_prefixed_secret_names() {
    let diagnostic = "OPENAI_API_KEY=sk-openai-live ANTHROPIC_API_KEY anthropic-live AZURE_ACCESS_TOKEN=azure-live safe-context";
    let redacted = crate::redaction::redact_diagnostic(diagnostic);

    for secret in ["sk-openai-live", "anthropic-live", "azure-live"] {
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
fn legacy_cost_notice_setting_is_read_and_rewritten_with_the_canonical_key() {
    let app_data = tempdir().unwrap();
    let settings_path = app_data.path().join("settings.json");
    fs::write(
        &settings_path,
        br#"{"ompExecutablePath":null,"theme":"system","selectedProviderId":null,"selectedModelId":null,"costNoticeAccepted":true}"#,
    )
    .unwrap();

    let service = AppService::new(settings_path.clone()).unwrap();
    assert!(
        service
            .get_ui_settings()
            .unwrap()
            .model_test_cost_notice_accepted
    );
    service
        .save_ui_settings(UiSettingsUpdate {
            theme: Theme::Dark,
            selected_provider_id: None,
            selected_model_id: None,
        })
        .unwrap();

    let persisted = fs::read_to_string(settings_path).unwrap();
    assert!(persisted.contains("modelTestCostNoticeAccepted"));
    assert!(!persisted.contains("costNoticeAccepted"));
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
        model_test_cost_notice_accepted: true,
    };

    service
        .save_ui_settings(UiSettingsUpdate {
            theme: Theme::Dark,
            selected_provider_id: Some("dnslin".to_owned()),
            selected_model_id: Some("gpt-5.6-sol".to_owned()),
        })
        .unwrap();
    service.accept_model_test_cost_notice().unwrap();
    service
        .save_ui_settings(UiSettingsUpdate {
            theme: Theme::Dark,
            selected_provider_id: Some("dnslin".to_owned()),
            selected_model_id: Some("gpt-5.6-sol".to_owned()),
        })
        .unwrap();

    assert_eq!(service.get_ui_settings().unwrap(), expected);

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
        "modelTestCostNoticeAccepted": false
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
      - id: gpt-5.6-sol
        name: Sol
        api: openai-responses
        reasoning: true
        input: [text, image]
        contextWindow: 356000
        maxTokens: 32768
        modelUnknown:
          nested: preserve-model
      - id: "gpt-5.6-sol:ultra"
        name: Ultra Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      - id: "gpt-5.6-sol:turbo"
        name: Turbo Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      - id: gpt-5.6-sol/high/extra
        name: Slash Model
        api: openai-responses
        input: [text]
        contextWindow: 356000
        maxTokens: 32768
      - id: incomplete
        name: Incomplete
        api: openai-responses
        reasoning: yes
        input: [text]
  other:
    baseUrl: https://example.com/v1?key=query-secret&sig=signed-url-secret&credential=credential-secret&x-goog-signature=goog-signature-secret&x-goog-credential=goog-credential-secret&public=display-me&region=us
    models:
      - id: mystery
        name: Mystery
        baseUrl: https://model-override.example/v1
        input: [audio]
  pathSecret:
    baseUrl: https://example.com/v1/sk-live-path-secret#fragment-secret
    models: []
  special:
    baseUrl: https:user:no-slashes-secret@example.com/v1
    models: []
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
  catalogOnly: aiand/deepseek-ai/deepseek-v4-flash
otherSettings:
  nested:
    value: untouched
"#;
    fs::write(target.join("models.yml"), models).unwrap();
    fs::write(target.join("config.yml"), config).unwrap();

    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let dto = serde_json::to_value(overview).unwrap();

    assert_eq!(dto["state"], "read-only");
    assert_eq!(dto["counts"]["providerCount"], 0);
    assert_eq!(dto["counts"]["modelCount"], 6);
    assert_eq!(dto["counts"]["roleCount"], 11);
    assert_eq!(dto["providers"][0]["hasApiKey"], true);
    assert_eq!(dto["providers"][0]["baseUrl"], "https://example.com/v1");
    assert_eq!(dto["providers"][0]["classification"], "advanced");
    assert_eq!(dto["providers"][0]["editable"], false);
    assert_eq!(
        dto["providers"][0]["readOnlyReason"],
        "包含 OMP Switch 不支持的高级配置。"
    );
    assert!(!dto.to_string().contains("super-secret-api-key"));
    assert!(!dto.to_string().contains("user-info-secret"));
    assert!(!dto.to_string().contains("query-secret"));
    assert!(!dto.to_string().contains("no-slashes-secret"));
    assert!(!dto.to_string().contains("signed-url-secret"));
    assert!(!dto.to_string().contains("credential-secret"));
    assert!(!dto.to_string().contains("goog-signature-secret"));
    assert!(!dto.to_string().contains("goog-credential-secret"));
    assert!(!dto.to_string().contains("display-me"));
    assert!(!dto.to_string().contains("path-secret"));
    assert!(!dto.to_string().contains("fragment-secret"));
    let serialized_load = serde_json::to_string(&service.get_overview_load()).unwrap();
    for secret in [
        "super-secret-api-key",
        "user-info-secret",
        "query-secret",
        "no-slashes-secret",
        "signed-url-secret",
        "credential-secret",
        "goog-signature-secret",
        "goog-credential-secret",
        "display-me",
        "path-secret",
        "fragment-secret",
        "apiKey",
    ] {
        assert!(!serialized_load.contains(secret), "IPC DTO leaked {secret}");
    }
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
    assert_eq!(provider("other")["classification"], "advanced");
    assert_eq!(provider("special")["classification"], "unsupported");
    assert_eq!(provider("dnslin")["baseUrl"], "https://example.com/v1");
    assert_eq!(provider("other")["baseUrl"], "[配置地址因无法解析而已脱敏]");
    let incomplete_model = provider("dnslin")["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "incomplete")
        .unwrap();
    assert_eq!(incomplete_model["complete"], false);
    assert_eq!(
        incomplete_model["readOnlyReason"],
        "Model definition 的 reasoning 字段格式不受支持。"
    );
    let mystery_model = provider("other")["models"]
        .as_array()
        .unwrap()
        .first()
        .unwrap();
    assert_eq!(mystery_model["input"][0], "unsupported");
    assert_eq!(mystery_model["hasBaseUrlOverride"], true);
    assert_eq!(
        provider("special")["baseUrl"],
        "[配置地址因无法解析而已脱敏]"
    );
    assert_eq!(
        provider("pathSecret")["baseUrl"],
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
        "catalogOnly",
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
    assert_eq!(role("catalogOnly")["status"], "configured");
    assert_eq!(role("catalogOnly")["providerId"], "aiand");
    assert_eq!(
        role("catalogOnly")["modelId"],
        "deepseek-ai/deepseek-v4-flash"
    );
    assert_eq!(
        role("catalogOnly")["thinkingLevel"],
        serde_json::Value::Null
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
fn overview_locks_malformed_and_unknown_role_selectors() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  standard:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: valid
        name: Valid
        api: openai-responses
        reasoning: true
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
      - id: ambiguous:high
        name: Ambiguous
        api: openai-responses
        reasoning: true
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  empty: \"\"\n  emptyThinking: standard/:high\n  padded: \" standard/valid\"\n  leadingSlash: /valid\n  trailingSlash: standard/\n  missingUnknown: standard/missing:ultra\n  missingMultiSlash: standard/missing/extra\n  ambiguous: standard/ambiguous:high\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let roles = dto["roles"].as_array().unwrap();
    let role = |id: &str| roles.iter().find(|role| role["id"] == id).unwrap();

    assert_eq!(dto["rolesEditable"], false);
    for id in ["padded", "leadingSlash", "trailingSlash", "missingUnknown"] {
        assert_eq!(role(id)["status"], "advanced", "role {id}");
        assert_eq!(role(id)["selector"], serde_json::Value::Null, "role {id}");
    }
    assert_eq!(role("empty")["status"], "unconfigured");
    assert_eq!(role("missingMultiSlash")["status"], "model-missing");
    assert_eq!(
        role("missingMultiSlash")["selector"],
        "standard/missing/extra"
    );
    assert_eq!(role("missingMultiSlash")["providerId"], "standard");
    assert_eq!(role("missingMultiSlash")["modelId"], "missing/extra");
    assert_eq!(
        role("missingMultiSlash")["thinkingLevel"],
        serde_json::Value::Null
    );
    assert_eq!(role("ambiguous")["status"], "model-missing");
    assert_eq!(role("ambiguous")["selector"], "standard/ambiguous:high");
    assert_eq!(role("ambiguous")["providerId"], "standard");
    assert_eq!(role("ambiguous")["modelId"], "ambiguous");
    assert_eq!(role("ambiguous")["thinkingLevel"], "high");
    assert_eq!(role("emptyThinking")["status"], "advanced");
    assert_eq!(role("emptyThinking")["selector"], serde_json::Value::Null);
}
#[test]
fn overview_locks_role_keys_with_invalid_spacing() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  standard:\n    baseUrl: https://example.com/v1\n    api: openai-responses\n    models:\n      - id: valid\n        name: Valid\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  \" padded\": standard/valid\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    assert_eq!(dto["rolesEditable"], false);
    assert_eq!(
        dto["rolesReadOnlyReason"],
        "config.yml 的 modelRoles 结构无法安全编辑。请在外部修复后重新读取。"
    );
}
#[test]
fn overview_role_structured_fields_do_not_leak_missing_selector_credentials() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  providerSecret: sk-live-secret/foo\n  modelSecret: standard/sk-live-secret\n  pathModelSecret: standard/foo/sk-live-secret\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = service.get_overview_load().overview.unwrap();
    let serialized = serde_json::to_string(&dto).unwrap();
    assert!(!serialized.contains("sk-live-secret"));
    for id in ["providerSecret", "modelSecret", "pathModelSecret"] {
        let role = dto.roles.iter().find(|role| role.id == id).unwrap();
        assert_eq!(role.provider_id, None);
        assert_eq!(role.model_id, None);
        assert_eq!(role.thinking_level, None);
    }
}

#[test]
fn overview_reads_standard_omp_model_definition_lists() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  standard:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: standard-model
        name: Standard Model
        api: openai-responses
        reasoning: true
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: standard/standard-model\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["state"], "normal");
    assert_eq!(dto["counts"]["providerCount"], 1);
    assert_eq!(dto["counts"]["modelCount"], 1);
    assert_eq!(dto["providers"][0]["classification"], "custom");
    assert_eq!(dto["providers"][0]["editable"], true);
    assert_eq!(dto["providers"][0]["modelCount"], 1);
    assert_eq!(dto["models"][0]["id"], "standard-model");
    assert_eq!(dto["models"][0]["complete"], true);
    assert_eq!(dto["models"][0]["editable"], true);
}

#[test]
fn overview_does_not_inherit_provider_api_past_unsupported_model_override() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  custom:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: unsupported-override
        name: Unsupported Override
        api: unsupported-custom-api
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/unsupported-override\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let model = &dto["models"][0];

    assert_eq!(model["effectiveApi"], serde_json::Value::Null);
    assert_eq!(model["apiSource"], serde_json::Value::Null);
    assert_eq!(model["editable"], false);
    assert_eq!(
        model["readOnlyReason"],
        "Model definition 使用了不支持的协议。"
    );
}

#[test]
fn overview_inherits_provider_api_for_null_model_api() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  custom:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: null-api
        name: Null API
        api: null
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/null-api\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let model = &dto["models"][0];

    assert_eq!(model["effectiveApi"], "openai-responses");
    assert_eq!(model["apiSource"], "provider");
    assert_eq!(model["complete"], true);
    assert_eq!(model["editable"], true);
    assert_eq!(model["readOnlyReason"], serde_json::Value::Null);
}

#[test]
fn overview_allows_null_provider_api_with_supported_model_override() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  custom:
    baseUrl: https://example.com/v1
    api: null
    models:
      - id: model-override
        name: Model Override
        api: openai-responses
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/model-override\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let provider = &dto["providers"][0];
    let model = &dto["models"][0];

    assert_eq!(dto["state"], "normal");
    assert_eq!(provider["defaultApi"], serde_json::Value::Null);
    assert_eq!(provider["classification"], "custom");
    assert_eq!(provider["editable"], true);
    assert_eq!(provider["readOnlyReason"], serde_json::Value::Null);
    assert_eq!(model["effectiveApi"], "openai-responses");
    assert_eq!(model["apiSource"], "model");
    assert_eq!(model["complete"], true);
    assert_eq!(model["editable"], true);
}

#[test]
fn overview_marks_case_insensitive_provider_and_model_collisions_read_only() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  Foo:
    baseUrl: https://foo.example/v1
    api: openai-responses
    models:
      - id: Model
        name: First
        input: [text]
        contextWindow: 100
        maxTokens: 10
      - id: model
        name: Second
        input: [text]
        contextWindow: 100
        maxTokens: 10
  foo:
    baseUrl: https://foo.example/v1
    api: openai-responses
    models:
      - id: other
        name: Other
        input: [text]
        contextWindow: 100
        maxTokens: 10
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: Foo/Model\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["state"], "read-only");
    for provider in dto["providers"].as_array().unwrap() {
        assert_eq!(provider["editable"], false);
        assert!(
            provider["readOnlyReason"]
                .as_str()
                .unwrap()
                .contains("不区分大小写")
        );
    }
    let foo = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "Foo")
        .unwrap();
    for model in foo["models"].as_array().unwrap() {
        assert_eq!(model["editable"], false);
        assert!(
            model["readOnlyReason"]
                .as_str()
                .unwrap()
                .contains("不区分大小写")
        );
    }
    assert_eq!(dto["roles"][0]["status"], "advanced");
}

#[test]
fn overview_excludes_unconfigured_roles_from_configured_role_count() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  custom:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: configured
        name: Configured
        input: [text]
        contextWindow: 128000
        maxTokens: 4096
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/configured\n  empty: ''\n  unset: null\n",
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["roles"].as_array().unwrap().len(), 3);
    assert_eq!(dto["counts"]["roleCount"], 1);
}

#[test]
fn overview_read_only_reason_identifies_unsupported_provider_shapes() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  unsupported:\n    baseUrl: ftp://example.com\n    models:\n      - id: local\n        name: Local\n        api: openai-responses\n        input: [text]\n        contextWindow: 100\n        maxTokens: 10\n",
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["state"], "read-only");
    assert_eq!(dto["providers"][0]["classification"], "unsupported");
    assert_eq!(
        dto["readOnlyReason"],
        "当前配置包含以下只读 Provider 分类：不支持的 Provider/Model 结构。"
    );
}

#[test]
fn overview_read_only_reason_enumerates_mixed_provider_classifications() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  openai:
    models:
      - id: gpt-5.6-sol
        name: Bundled
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  advanced:
    baseUrl: https://example.com/v1
    headers:
      x-test: value
    models:
      - id: advanced-model
        name: Advanced
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  unsupported:
    baseUrl: ftp://example.com
    models:
      - id: unsupported-model
        name: Unsupported
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
"#,
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();

    assert_eq!(dto["state"], "read-only");
    assert_eq!(
        dto["readOnlyReason"],
        "当前配置包含以下只读 Provider 分类：OMP 内置 Provider/Model 覆盖、高级 Provider、不支持的 Provider/Model 结构。"
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
      - id: gpt-5.6-sol
        name: Bundled
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  missing:
    models:
      - id: local
        name: Missing URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  malformed:
    baseUrl: ftp://example.com
    models:
      - id: local
        name: Malformed URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  nonString:
    baseUrl: 42
    models:
      - id: local
        name: Non-string URL
        api: openai-responses
        input: [text]
        contextWindow: 100
        maxTokens: 10
  empty:
    models: []
  custom:
    baseUrl: https://example.com
    models:
      - id: local
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
fn overview_dto_redacts_embedded_target_diagnostics() {
    let app_data = tempdir().unwrap().keep();
    let target_path = app_data.join("agent");
    fs::create_dir_all(&target_path).unwrap();
    fs::write(target_path.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target_path.join("config.yml"), "modelRoles: {}\n").unwrap();
    let mut target = writable_target(&target_path);
    target.recovery_notice = Some("recovered sk-live-recovery".to_owned());
    target.warnings = vec!["warning token=sk-live-warning".to_owned()];
    target.issue = Some(ConfigurationIssue {
        file_path: target_path.join("models.yml").display().to_string(),
        line: Some(3),
        column: Some(1),
        message: "issue Authorization: Bearer sk-live-issue".to_owned(),
    });

    let dto = crate::overview::read_overview("/bin/omp", "17.3.4", &target)
        .unwrap()
        .dto;
    let serialized = serde_json::to_string(&dto).unwrap();
    for secret in ["sk-live-recovery", "sk-live-warning", "sk-live-issue"] {
        assert!(!serialized.contains(secret), "Overview DTO leaked {secret}");
    }
}

#[test]
fn overview_load_redacts_target_configuration_diagnostics_at_ipc_seam() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let mut discovery = writable_target(&target);
    discovery.status = TargetConfigurationStatus::ParseError;
    discovery.issue = Some(ConfigurationIssue {
        file_path: target.join("models.yml").display().to_string(),
        line: Some(18),
        column: Some(4),
        message: "parse failed at https://example.test/v1/sk-live-abc123".to_owned(),
    });
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        target_override: Some(discovery),
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);

    let load = service.get_overview_load();
    assert_eq!(
        load.error.as_ref().map(|error| error.code),
        Some("overview-parse-error")
    );
    assert!(load.overview.is_none());
    let serialized = serde_json::to_string(&load).unwrap();
    assert!(!serialized.contains("sk-live-abc123"));
    assert!(serialized.contains("[诊断信息因可能包含凭据而已脱敏]"));
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

fn provider_mutation_service(target: &Path, app_data: &Path) -> AppService {
    fs::create_dir_all(app_data).unwrap();
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.to_path_buf()),
        inspect_real_target: true,
        transaction_root: app_data.join("target-initialization-transactions"),
        ..FakeOmpEnvironment::default()
    });
    AppService::new_with_environment(app_data.join("settings.json"), environment).unwrap()
}

fn provider_creation_input(opened_models_hash: String) -> CreateCustomProviderInput {
    CreateCustomProviderInput {
        opened_models_hash,
        provider: CreateProviderFields {
            id: "  new-provider  ".to_owned(),
            base_url: " https://api.new-provider.example/v1/ ".to_owned(),
            default_api: Some(SupportedApi::OpenAiResponses),
            auth_mode: ProviderAuthMode::ApiKey,
            api_key: Some("create-secret-must-not-leak".to_owned()),
        },
        first_model: CreateModelFields {
            id: "  new-model  ".to_owned(),
            name: "New Model".to_owned(),
            api: None,
            reasoning: true,
            input: vec![SupportedInput::Text, SupportedInput::Image],
            context_window: 128_000,
            max_tokens: 8_192,
        },
    }
}

fn provider_edit_input(
    opened_models_hash: String,
    api_key: DirectApiKeyIntent,
) -> EditCustomProviderInput {
    EditCustomProviderInput {
        opened_models_hash,
        provider_id: "editable".to_owned(),
        base_url: " https://edited.example/v1/ ".to_owned(),
        default_api: Some(SupportedApi::AnthropicMessages),
        auth_mode: ProviderAuthMode::ApiKey,
        api_key,
    }
}

fn editable_provider_yaml(api_key: Option<&str>) -> String {
    let api_key = api_key
        .map(|value| format!("    apiKey: {value}\n"))
        .unwrap_or_default();
    format!(
        "providers:\n  editable:\n    baseUrl: https://original.example/v1\n    api: openai-completions\n{api_key}    models:\n      - id: editable-model\n        name: Editable Model\n        reasoning: false\n        input: [text]\n        contextWindow: 4096\n        maxTokens: 1024\n"
    )
}

fn opened_models_hash(service: &AppService) -> String {
    service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap()
}

#[test]
fn provider_edit_replaces_supported_fields_without_leaking_the_direct_api_key() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = r#"unrecognizedRoot:
  nested:
    value: preserve-root
providers:
  editable:
    name: Editable Provider
    baseUrl: https://original.example/v1
    api: openai-completions
    apiKey: fixture-old-direct-key
    models:
      - id: editable-model
        name: Editable Model
        reasoning: false
        input: [text]
        contextWindow: 4096
        maxTokens: 1024
        futureModelSetting:
          value: preserve-target-descendant
  sibling:
    baseUrl: https://sibling.example/v1
    api: openai-responses
    models:
      - id: sibling-model
        name: Sibling Model
        reasoning: false
        input: [text]
        contextWindow: 4096
        maxTokens: 1024
"#;
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();
    let replacement = "fixture-replacement-direct-key";

    let result = service
        .edit_custom_provider(provider_edit_input(
            opened_models_hash,
            DirectApiKeyIntent::Replace {
                value: replacement.to_owned(),
            },
        ))
        .unwrap();

    assert_eq!(result.provider_id, "editable");
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains(replacement)
    );
    let before: serde_yaml::Value = serde_yaml::from_str(original).unwrap();
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(after["unrecognizedRoot"], before["unrecognizedRoot"]);
    assert_eq!(
        after["providers"]["sibling"],
        before["providers"]["sibling"]
    );
    assert_eq!(
        after["providers"]["editable"]["models"],
        before["providers"]["editable"]["models"]
    );
    assert_eq!(
        after["providers"]["editable"]["baseUrl"],
        "https://edited.example/v1"
    );
    assert_eq!(after["providers"]["editable"]["api"], "anthropic-messages");
    let written_key = after["providers"]["editable"]["apiKey"].as_str().unwrap();
    assert_eq!(
        Sha256::digest(written_key.as_bytes()),
        Sha256::digest(replacement.as_bytes())
    );
    let overview_json =
        serde_json::to_string(&service.get_overview_load().overview.unwrap()).unwrap();
    assert!(!overview_json.contains(replacement));
}

#[test]
fn provider_edit_keeps_an_existing_direct_api_key() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_key = "fixture-direct-key-to-keep";
    fs::write(
        target.join("models.yml"),
        editable_provider_yaml(Some(original_key)),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());

    service
        .edit_custom_provider(provider_edit_input(
            opened_models_hash(&service),
            DirectApiKeyIntent::Keep,
        ))
        .unwrap();

    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    let key = after["providers"]["editable"]["apiKey"].as_str().unwrap();
    assert_eq!(
        Sha256::digest(key.as_bytes()),
        Sha256::digest(original_key.as_bytes())
    );
    assert!(
        !serde_json::to_string(&service.get_overview_load().overview.unwrap())
            .unwrap()
            .contains(original_key)
    );
}

#[test]
fn provider_edit_deletes_the_direct_api_key_for_no_authentication() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_key = "fixture-direct-key-to-delete";
    fs::write(
        target.join("models.yml"),
        editable_provider_yaml(Some(original_key)),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let mut input = provider_edit_input(opened_models_hash(&service), DirectApiKeyIntent::Delete);
    input.auth_mode = ProviderAuthMode::None;

    let result = service.edit_custom_provider(input).unwrap();

    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains(original_key)
    );
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert!(
        !after["providers"]["editable"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::String("apiKey".to_owned()))
    );
    let overview = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let provider = overview["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "editable")
        .unwrap();
    assert_eq!(provider["authMode"], "none");
    assert_eq!(provider["hasApiKey"], false);
    assert!(
        !serde_json::to_string(provider)
            .unwrap()
            .contains(original_key)
    );
}

#[test]
fn provider_edit_rejects_masked_or_command_key_replacements_without_leaking_them() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = editable_provider_yaml(Some("fixture-existing-direct-key"));
    fs::write(target.join("models.yml"), &original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    for replacement in [
        "••••••",
        "!fixture-command-key-must-not-leak",
        " !fixture-command-key-must-not-leak",
    ] {
        let error = service
            .edit_custom_provider(provider_edit_input(
                opened_models_hash(&service),
                DirectApiKeyIntent::Replace {
                    value: replacement.to_owned(),
                },
            ))
            .unwrap_err();
        assert_eq!(error.code, "provider-api-key-invalid");
        assert!(!serde_json::to_string(&error).unwrap().contains(replacement));
        assert_eq!(
            Sha256::digest(fs::read(target.join("models.yml")).unwrap()),
            Sha256::digest(original.as_bytes()),
        );
    }
}

#[test]
fn provider_edit_stops_on_an_external_models_hash_conflict() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        editable_provider_yaml(Some("fixture-key")),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = opened_models_hash(&service);
    let externally_changed = b"unrecognizedRoot:\n  changed: outside-omp-switch\nproviders: {}\n";
    fs::write(target.join("models.yml"), externally_changed).unwrap();

    let error = service
        .edit_custom_provider(provider_edit_input(
            opened_models_hash,
            DirectApiKeyIntent::Keep,
        ))
        .unwrap_err();

    assert_eq!(error.code, "models-hash-conflict");
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        externally_changed
    );
    assert!(
        !app_data
            .path()
            .join("target-configuration-backups")
            .exists()
    );
}

#[test]
fn provider_edit_failure_injection_keeps_the_original_file_intact() {
    let mut failures = vec![
        ("before backup", ModelsWriteFailurePoint::BeforeBackup),
        (
            "backup directory",
            ModelsWriteFailurePoint::BackupDirectoryCreationFailure,
        ),
        (
            "backup file open",
            ModelsWriteFailurePoint::BackupFileOpenFailure,
        ),
        (
            "backup file write",
            ModelsWriteFailurePoint::BackupFileWriteFailure,
        ),
        (
            "backup file sync",
            ModelsWriteFailurePoint::BackupFileSyncFailure,
        ),
        (
            "temporary open",
            ModelsWriteFailurePoint::TemporaryFileOpenFailure,
        ),
        (
            "temporary write",
            ModelsWriteFailurePoint::TemporaryFileWriteFailure,
        ),
        (
            "temporary sync",
            ModelsWriteFailurePoint::TemporaryFileSyncFailure,
        ),
        ("after backup", ModelsWriteFailurePoint::AfterBackup),
        (
            "before temporary write",
            ModelsWriteFailurePoint::BeforeTemporaryWrite,
        ),
        (
            "temporary reparse",
            ModelsWriteFailurePoint::CorruptTemporaryFile,
        ),
        (
            "untouched comparison",
            ModelsWriteFailurePoint::MutateUntouchedValue,
        ),
        (
            "before replacement",
            ModelsWriteFailurePoint::BeforeReplacement,
        ),
    ];
    #[cfg(unix)]
    failures.push(("replacement commit", ModelsWriteFailurePoint::CommitFailure));
    for (name, failure) in failures {
        let app_data = tempdir().unwrap();
        let target = app_data.path().join("agent");
        fs::create_dir_all(&target).unwrap();
        let original = editable_provider_yaml(Some("fixture-failure-direct-key"));
        fs::write(target.join("models.yml"), &original).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
        let service = provider_mutation_service(&target, app_data.path());
        let input = provider_edit_input(
            opened_models_hash(&service),
            DirectApiKeyIntent::Replace {
                value: "fixture-replacement-for-injection".to_owned(),
            },
        );
        service.set_models_write_failure_for_test(failure);

        let error = service.edit_custom_provider(input).unwrap_err();

        assert_eq!(error.code, "provider-edit-failed", "{name}");
        assert_eq!(
            Sha256::digest(fs::read(target.join("models.yml")).unwrap()),
            Sha256::digest(original.as_bytes()),
            "{name}",
        );
    }
}

#[test]
fn provider_edit_refuses_a_base_url_that_was_redacted_from_the_projection() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = b"providers:\n  editable:\n    baseUrl: https://fixture-user:fixture-base-url-password@example.com/v1\n    api: openai-responses\n    apiKey: fixture-existing-direct-key\n    models:\n      - id: editable-model\n        name: Editable Model\n        reasoning: false\n        input: [text]\n        contextWindow: 4096\n        maxTokens: 1024\n";
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();
    let opened_models_hash = overview.files.models.content_hash.clone().unwrap();
    let projected = serde_json::to_string(&overview).unwrap();

    assert!(!projected.contains("fixture-user"));
    assert!(!projected.contains("fixture-base-url-password"));
    assert!(!overview.providers[0].editable);
    let error = service
        .edit_custom_provider(provider_edit_input(
            opened_models_hash,
            DirectApiKeyIntent::Keep,
        ))
        .unwrap_err();

    assert_eq!(error.code, "provider-edit-unavailable");
    assert_eq!(
        Sha256::digest(fs::read(target.join("models.yml")).unwrap()),
        Sha256::digest(original),
    );
}

#[test]
fn provider_overview_does_not_expose_a_secret_in_a_safe_named_base_url_query() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let secret = format!("{}{}", "sk-", "fixture-query-secret");
    let base_url = format!("https://original.example/v1?project={secret}");
    let original = format!(
        "providers:\n  editable:\n    baseUrl: {base_url}\n    api: openai-responses\n    apiKey: fixture-existing-direct-key\n    models:\n      - id: editable-model\n        name: Editable Model\n        reasoning: false\n        input: [text]\n        contextWindow: 4096\n        maxTokens: 1024\n"
    );
    fs::write(target.join("models.yml"), &original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();
    let overview_json = serde_json::to_string(&overview).unwrap();

    assert!(!overview_json.contains(&secret));
    assert!(!overview.providers[0].editable);
}

#[test]
fn provider_edit_preserves_an_explicit_default_port_in_base_url() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_base_url = "https://original.example:443/v1";
    let original = editable_provider_yaml(Some("fixture-existing-direct-key")).replacen(
        "https://original.example/v1",
        original_base_url,
        1,
    );
    fs::write(target.join("models.yml"), &original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();
    let mut input = provider_edit_input(
        overview.files.models.content_hash.clone().unwrap(),
        DirectApiKeyIntent::Keep,
    );
    input.base_url = overview.providers[0].base_url.clone().unwrap();

    assert_eq!(
        overview.providers[0].base_url.as_deref(),
        Some(original_base_url)
    );
    assert!(overview.providers[0].editable);
    service.edit_custom_provider(input).unwrap();

    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(
        after["providers"]["editable"]["baseUrl"].as_str(),
        Some(original_base_url)
    );
}

#[test]
fn command_credential_has_no_mutation_capability_in_the_overview_dto() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = b"providers:\n  restricted:\n    baseUrl: https://original.example/v1\n    api: openai-responses\n    apiKey: !command fixture-command-credential\n    models:\n      - id: restricted-model\n        name: Restricted Model\n        reasoning: false\n        input: [text]\n        contextWindow: 4096\n        maxTokens: 1024\n";
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();
    let overview_json = serde_json::to_value(overview).unwrap();

    assert!(!overview_json["providers"][0]["editable"].as_bool().unwrap());
    assert!(
        overview_json["providers"][0]
            .get("canReplaceCommandCredential")
            .is_none()
    );
}

#[test]
fn custom_provider_creation_preserves_deep_unknown_values_and_creates_a_current_backup() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = r#"unrecognizedRoot:
  nested:
    branches:
      - leaf: untouched
providers:
  legacy:
    baseUrl: https://legacy.example/v1
    api: openai-responses
    models:
      - id: legacy-model
        name: Legacy Model
        reasoning: false
        input: [text]
        contextWindow: 4096
        maxTokens: 1024
"#;
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();

    let result = service
        .create_custom_provider(provider_creation_input(opened_models_hash))
        .unwrap();

    assert_eq!(result.provider_id, "new-provider");
    assert_eq!(result.model_id, "new-model");
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("create-secret-must-not-leak")
    );

    let before: serde_yaml::Value = serde_yaml::from_str(original).unwrap();
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(after["unrecognizedRoot"], before["unrecognizedRoot"]);
    assert_eq!(after["providers"]["legacy"], before["providers"]["legacy"]);
    assert_eq!(
        after["providers"]["new-provider"]["baseUrl"],
        "https://api.new-provider.example/v1"
    );
    assert_eq!(
        after["providers"]["new-provider"]["apiKey"],
        "create-secret-must-not-leak"
    );
    assert_eq!(
        after["providers"]["new-provider"]["models"][0]["id"],
        "new-model"
    );

    let backup_root = app_data.path().join("target-configuration-backups");
    let lock_directory = backup_root.join(".locks");
    let backup_targets = fs::read_dir(&backup_root)
        .unwrap()
        .map(Result::unwrap)
        .filter(|entry| entry.file_name() != ".locks")
        .collect::<Vec<_>>();
    assert_eq!(backup_targets.len(), 1);
    let model_backup_directory = backup_targets[0].path().join("models.yml");
    let model_backups = fs::read_dir(&model_backup_directory)
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(model_backups.len(), 1);
    assert_eq!(
        fs::read(model_backups[0].path()).unwrap(),
        original.as_bytes()
    );
    let lock_files = fs::read_dir(&lock_directory)
        .unwrap()
        .map(Result::unwrap)
        .collect::<Vec<_>>();
    assert_eq!(lock_files.len(), 1);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mode =
            |path: &std::path::Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&backup_root), 0o700);
        assert_eq!(mode(&lock_directory), 0o700);
        assert_eq!(mode(&lock_files[0].path()), 0o600);
        assert_eq!(mode(&backup_targets[0].path()), 0o700);
        assert_eq!(mode(&model_backup_directory), 0o700);
        assert_eq!(mode(&model_backups[0].path()), 0o600);
    }

    let refreshed = service.get_overview_load().overview.unwrap();
    assert!(
        refreshed
            .providers
            .iter()
            .any(|provider| provider.id == "new-provider" && provider.editable)
    );
}

#[test]
fn custom_provider_creation_preserves_api_key_mode_without_direct_key() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();
    let mut input = provider_creation_input(opened_models_hash);
    input.provider.api_key = None;

    service.create_custom_provider(input).unwrap();

    let created: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(created["providers"]["new-provider"]["apiKey"], "");
    let refreshed = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let provider = refreshed["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "new-provider")
        .unwrap();
    assert_eq!(provider["authMode"], "api-key");
    assert_eq!(provider["hasApiKey"], false);
}

#[test]
fn custom_provider_creation_accepts_spec_valid_model_suffix_and_limits() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();
    let mut input = provider_creation_input(opened_models_hash);
    input.first_model.id = "  new-model:high  ".to_owned();
    input.first_model.context_window = 1_024;
    input.first_model.max_tokens = 512;

    let result = service.create_custom_provider(input).unwrap();

    assert_eq!(result.model_id, "new-model:high");
    let created: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    let model = &created["providers"]["new-provider"]["models"][0];
    assert_eq!(model["id"], "new-model:high");
    assert_eq!(model["contextWindow"], 1_024);
    assert_eq!(model["maxTokens"], 512);
    let refreshed = service.get_overview_load().overview.unwrap();
    let model = refreshed
        .models
        .iter()
        .find(|model| model.id == "new-model:high")
        .unwrap();
    assert!(model.complete);
    assert!(model.editable);
}

#[test]
fn custom_provider_creation_rejects_a_changed_models_hash_before_creating_a_backup() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();
    let externally_changed = b"unrecognizedRoot:\n  changed: outside-omp-switch\nproviders: {}\n";
    fs::write(target.join("models.yml"), externally_changed).unwrap();

    let error = service
        .create_custom_provider(provider_creation_input(opened_models_hash))
        .unwrap_err();

    assert_eq!(error.code, "models-hash-conflict");
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        externally_changed
    );
    assert!(
        !app_data
            .path()
            .join("target-configuration-backups")
            .exists()
    );
}

#[test]
fn custom_provider_creation_stops_when_another_writer_holds_the_target_lock() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = b"providers: {}\n";
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let resolved_target = fs::canonicalize(&target).unwrap();
    let fingerprint = Sha256::digest(resolved_target.to_string_lossy().as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let lock_directory = app_data
        .path()
        .join("target-configuration-backups")
        .join(".locks");
    fs::create_dir_all(&lock_directory).unwrap();
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_directory.join(format!("{fingerprint}.lock")))
        .unwrap();
    FileExt::lock(&lock).unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();

    let error = service
        .create_custom_provider(provider_creation_input(opened_models_hash))
        .unwrap_err();

    assert_eq!(error.code, "models-write-in-progress");
    assert_eq!(fs::read(target.join("models.yml")).unwrap(), original);
}

#[test]
fn custom_provider_creation_rejects_invalid_and_colliding_values_without_writing() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = b"providers:\n  existing:\n    baseUrl: https://existing.example/v1\n    api: openai-responses\n    models:\n      - id: existing-model\n        name: Existing model\n        reasoning: false\n        input: [text]\n        contextWindow: 4096\n        maxTokens: 1024\n";
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();

    let invalid_cases = [
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.provider.id = "not a provider".to_owned();
            ("invalid Provider ID", input, "provider-id-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.provider.id = "EXISTING".to_owned();
            ("duplicate Provider ID", input, "provider-id-conflict")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.provider.base_url = "ftp://invalid.example".to_owned();
            ("invalid Base URL", input, "provider-base-url-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.provider.auth_mode = ProviderAuthMode::None;
            input.provider.api_key = Some("stale-key".to_owned());
            (
                "API Key with disabled authentication",
                input,
                "provider-auth-invalid",
            )
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.provider.api_key = Some("!command credential".to_owned());
            ("command API Key", input, "provider-api-key-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.first_model.id = "invalid model".to_owned();
            ("invalid Model ID", input, "model-id-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.first_model.name = "  ".to_owned();
            ("missing Model name", input, "model-name-required")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.first_model.input.clear();
            ("missing Model capability", input, "model-input-required")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.first_model.context_window = 0;
            ("zero Context Window", input, "model-context-window-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash.clone());
            input.first_model.max_tokens = 0;
            ("zero Max Tokens", input, "model-token-limit-invalid")
        },
        {
            let mut input = provider_creation_input(opened_models_hash);
            input.provider.default_api = None;
            input.first_model.api = None;
            ("missing Model protocol", input, "model-api-required")
        },
    ];
    for (name, input, expected_code) in invalid_cases {
        let error = service.create_custom_provider(input).unwrap_err();
        assert_eq!(error.code, expected_code, "{name}");
        assert_eq!(
            fs::read(target.join("models.yml")).unwrap(),
            original,
            "{name}"
        );
    }
}

#[test]
fn custom_provider_creation_failure_injection_keeps_the_original_file_intact() {
    let mut failures = vec![
        ("before backup", ModelsWriteFailurePoint::BeforeBackup),
        (
            "backup directory creation",
            ModelsWriteFailurePoint::BackupDirectoryCreationFailure,
        ),
        (
            "backup file open",
            ModelsWriteFailurePoint::BackupFileOpenFailure,
        ),
        (
            "backup file write",
            ModelsWriteFailurePoint::BackupFileWriteFailure,
        ),
        (
            "backup file sync",
            ModelsWriteFailurePoint::BackupFileSyncFailure,
        ),
        (
            "temporary file open",
            ModelsWriteFailurePoint::TemporaryFileOpenFailure,
        ),
        (
            "temporary file write",
            ModelsWriteFailurePoint::TemporaryFileWriteFailure,
        ),
        (
            "temporary file sync",
            ModelsWriteFailurePoint::TemporaryFileSyncFailure,
        ),
        ("after backup", ModelsWriteFailurePoint::AfterBackup),
        (
            "before temporary write",
            ModelsWriteFailurePoint::BeforeTemporaryWrite,
        ),
        (
            "temporary reparse",
            ModelsWriteFailurePoint::CorruptTemporaryFile,
        ),
        (
            "untouched comparison",
            ModelsWriteFailurePoint::MutateUntouchedValue,
        ),
        (
            "before replacement",
            ModelsWriteFailurePoint::BeforeReplacement,
        ),
    ];
    #[cfg(unix)]
    failures.push((
        "replacement commit failure",
        ModelsWriteFailurePoint::CommitFailure,
    ));
    for (name, failure) in failures {
        let app_data = tempdir().unwrap();
        let target = app_data.path().join("agent");
        fs::create_dir_all(&target).unwrap();
        let original = b"unrecognizedRoot:\n  nested:\n    value: preserve-me\nproviders: {}\n";
        fs::write(target.join("models.yml"), original).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
        let service = provider_mutation_service(&target, app_data.path());
        let opened_models_hash = service
            .get_overview_load()
            .overview
            .unwrap()
            .files
            .models
            .content_hash
            .unwrap();
        service.set_models_write_failure_for_test(failure);

        assert!(
            service
                .create_custom_provider(provider_creation_input(opened_models_hash))
                .is_err(),
            "{name}"
        );
        assert_eq!(
            fs::read(target.join("models.yml")).unwrap(),
            original,
            "{name}"
        );
    }
}

#[test]
fn custom_provider_creation_does_not_report_failure_after_replacing_models() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = b"unrecognizedRoot:\n  nested:\n    value: preserve-me\nproviders: {}\n";
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_models_hash = service
        .get_overview_load()
        .overview
        .unwrap()
        .files
        .models
        .content_hash
        .unwrap();
    service.set_models_write_failure_for_test(ModelsWriteFailurePoint::AfterAtomicReplacement);

    let result = service.create_custom_provider(provider_creation_input(opened_models_hash));
    let written = fs::read(target.join("models.yml")).unwrap();
    match result {
        Ok(_) => assert_ne!(
            written, original,
            "a successful creation must write the candidate"
        ),
        Err(_) => assert_eq!(
            written, original,
            "a failed creation must leave models.yml unchanged"
        ),
    }
}

#[cfg(unix)]
#[test]
fn custom_provider_creation_reports_a_partial_backup_cleanup_failure() {
    let warnings = Arc::new(AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(CleanupWarningCounter(warnings.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let app_data = tempdir().unwrap();
        let target = app_data.path().join("agent");
        fs::create_dir_all(&target).unwrap();
        let original = b"unrecognizedRoot:\n  nested:\n    value: preserve-me\nproviders: {}\n";
        fs::write(target.join("models.yml"), original).unwrap();
        fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
        let service = provider_mutation_service(&target, app_data.path());
        let opened_models_hash = service
            .get_overview_load()
            .overview
            .unwrap()
            .files
            .models
            .content_hash
            .unwrap();
        service.set_models_write_failure_for_test(
            ModelsWriteFailurePoint::BackupFilePermissionAndCleanupFailure,
        );

        let error = service
            .create_custom_provider(provider_creation_input(opened_models_hash))
            .unwrap_err();
        assert_eq!(error.code, "provider-create-failed");
        assert_eq!(fs::read(target.join("models.yml")).unwrap(), original);
    });
    assert_eq!(warnings.load(Ordering::SeqCst), 1);
}
#[test]
fn model_create_and_edit_preserve_unknown_fields_and_stable_protocol_semantics() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"unrecognizedRoot:
  keep: root
providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: existing
        name: Existing
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: sibling
        name: Sibling
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
        modelUnknown:
          keep: model
  other:
    baseUrl: https://other.example/v1
    api: openai-responses
    providerUnknown:
      keep: provider
    models:
      - id: other-model
        name: Other
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let opened_hash = opened_models_hash(&service);

    let created = service
        .create_model(CreateModelInput {
            opened_models_hash: opened_hash,
            provider_id: "editable".to_owned(),
            model: ModelDefinitionFields {
                id: "  created  ".to_owned(),
                name: "Created".to_owned(),
                api: Some(SupportedApi::AnthropicMessages),
                reasoning: true,
                input: vec![SupportedInput::Text, SupportedInput::Image],
                context_window: 200_000,
                max_tokens: 20_000,
            },
        })
        .unwrap();
    assert_eq!(created.model_id, "created");

    let created_tree: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(created_tree["unrecognizedRoot"]["keep"], "root");
    assert_eq!(
        created_tree["providers"]["other"]["providerUnknown"]["keep"],
        "provider"
    );
    assert_eq!(
        created_tree["providers"]["editable"]["models"][2]["api"],
        "anthropic-messages"
    );
    assert_eq!(
        created_tree["providers"]["editable"]["models"][1]["modelUnknown"]["keep"],
        "model"
    );

    let edited = service
        .edit_model(EditModelInput {
            opened_models_hash: opened_models_hash(&service),
            provider_id: "editable".to_owned(),
            model_id: "existing".to_owned(),
            model: ModelEditFields {
                name: "Edited".to_owned(),
                api: None,
                reasoning: false,
                input: vec![SupportedInput::Image],
                context_window: 120_000,
                max_tokens: 12_000,
            },
        })
        .unwrap();
    assert_eq!(edited.model_id, "existing");

    let edited_tree: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    let existing = &edited_tree["providers"]["editable"]["models"][0];
    assert_eq!(existing["name"], "Edited");
    assert_eq!(existing["api"], serde_yaml::Value::Null);
    assert_eq!(
        edited_tree["providers"]["editable"]["models"][1]["modelUnknown"]["keep"],
        "model"
    );
    assert_eq!(
        existing["input"],
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::String("image".to_owned())])
    );
}

#[test]
fn model_copy_creates_a_new_definition_from_supported_fields_only() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  editable:\n    baseUrl: https://example.com/v1\n    api: openai-responses\n    models:\n      - id: source\n        name: Source\n        input: [text, image]\n        reasoning: true\n        contextWindow: 100000\n        maxTokens: 10000\n",
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let result = service
        .create_model(CreateModelInput {
            opened_models_hash: opened_models_hash(&service),
            provider_id: "editable".to_owned(),
            model: ModelDefinitionFields {
                id: "source-copy".to_owned(),
                name: "Source Copy".to_owned(),
                api: None,
                reasoning: true,
                input: vec![SupportedInput::Text, SupportedInput::Image],
                context_window: 100_000,
                max_tokens: 10_000,
            },
        })
        .unwrap();
    assert_eq!(result.model_id, "source-copy");
    let tree: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    let copied = &tree["providers"]["editable"]["models"][1];
    assert_eq!(copied["id"], "source-copy");
    assert_eq!(copied["name"], "Source Copy");
    assert_eq!(copied["api"], serde_yaml::Value::Null);
    assert_eq!(
        copied
            .as_mapping()
            .unwrap()
            .get(serde_yaml::Value::String("testResult".to_owned())),
        None
    );
}
#[test]
fn model_mutations_enforce_validation_stable_ids_and_read_only_boundaries() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: existing
        name: Existing
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: locked
        name: Locked
        baseUrl: https://model.example/v1
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let hash = opened_models_hash(&service);

    let mut duplicate = CreateModelInput {
        opened_models_hash: hash.clone(),
        provider_id: "editable".to_owned(),
        model: ModelDefinitionFields {
            id: "EXISTING".to_owned(),
            name: "Duplicate".to_owned(),
            api: None,
            reasoning: false,
            input: vec![SupportedInput::Text],
            context_window: 100_000,
            max_tokens: 10_000,
        },
    };
    assert_eq!(
        service.create_model(duplicate.clone()).unwrap_err().code,
        "model-id-conflict"
    );
    duplicate.model.id = "new:auto".to_owned();
    assert_eq!(
        service.create_model(duplicate.clone()).unwrap_err().code,
        "model-id-invalid"
    );
    duplicate.model.id = "new-model".to_owned();
    duplicate.model.max_tokens = 200_000;
    assert_eq!(
        service.create_model(duplicate).unwrap_err().code,
        "model-token-limit-invalid"
    );

    let stable_error = service
        .edit_model(EditModelInput {
            opened_models_hash: hash.clone(),
            provider_id: "editable".to_owned(),
            model_id: "EXISTING".to_owned(),
            model: ModelEditFields {
                name: "Changed".to_owned(),
                api: None,
                reasoning: false,
                input: vec![SupportedInput::Text],
                context_window: 100_000,
                max_tokens: 10_000,
            },
        })
        .unwrap_err();
    assert_eq!(stable_error.code, "model-id-immutable");

    let read_only_edit = service
        .edit_model(EditModelInput {
            opened_models_hash: hash.clone(),
            provider_id: "editable".to_owned(),
            model_id: "locked".to_owned(),
            model: ModelEditFields {
                name: "Changed".to_owned(),
                api: None,
                reasoning: false,
                input: vec![SupportedInput::Text],
                context_window: 100_000,
                max_tokens: 10_000,
            },
        })
        .unwrap_err();
    assert_eq!(read_only_edit.code, "model-read-only");

    let overview = service.get_overview_load().overview.unwrap();
    let read_only_delete = service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "locked".to_owned(),
        })
        .unwrap_err();
    assert_eq!(read_only_delete.code, "model-read-only");
}
#[test]
fn overview_classifies_incomplete_and_read_only_models_and_counts_full_config_references() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: incomplete
        name: ""
        input: []
        contextWindow: 0
        maxTokens: 0
      - id: locked
        name: Locked
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
        baseUrl: https://model.example/v1
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        r#"modelRoles:
  default: editable/locked
otherSettings:
  fallback: editable/locked:high
  "api_key=fixture-reference-secret": editable/locked
"#,
    )
    .unwrap();

    let service = service_for_target(&target);
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let incomplete = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "incomplete")
        .unwrap();
    let locked = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "locked")
        .unwrap();
    assert_eq!(incomplete["status"], "incomplete");
    assert_eq!(incomplete["complete"], false);
    assert_eq!(incomplete["editable"], true);
    assert_eq!(locked["status"], "read-only");
    assert_eq!(locked["editable"], false);
    assert_eq!(locked["referenceCount"], 3);
    assert_eq!(locked["referencePaths"].as_array().unwrap().len(), 3);
    assert!(!dto.to_string().contains("fixture-reference-secret"));
}

#[test]
fn model_delete_requires_no_reference_and_never_removes_the_last_model() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"unrecognizedRoot:
  keep: root
providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let deleted = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "second".to_owned(),
        })
        .unwrap();
    assert_eq!(deleted.model_id, "second");
    let after_delete: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(after_delete["unrecognizedRoot"]["keep"], "root");
    assert_eq!(
        after_delete["providers"]["editable"]["models"]
            .as_sequence()
            .unwrap()
            .len(),
        1
    );
    let models_before_unknown_suffix = fs::read(target.join("models.yml")).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  ambiguous: EDITABLE/FIRST:ultra\n",
    )
    .unwrap();
    let unknown_suffix = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();
    assert_eq!(unknown_suffix.code, "model-delete-unmanaged-reference");
    assert!(
        unknown_suffix
            .message
            .contains("config.yml:modelRoles[\"ambiguous\"]")
    );
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        models_before_unknown_suffix
    );
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  candidates: editable/first,editable/second\n",
    )
    .unwrap();
    let comma_blocked = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();
    assert_eq!(comma_blocked.code, "model-delete-unmanaged-reference");
    assert!(
        comma_blocked
            .message
            .contains("config.yml:modelRoles[\"candidates\"]")
    );
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  tagged: !selector editable/first\n",
    )
    .unwrap();
    let tagged_blocked = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();
    assert_eq!(tagged_blocked.code, "model-delete-unmanaged-reference");
    assert!(
        tagged_blocked
            .message
            .contains("config.yml:modelRoles[\"tagged\"]")
    );

    fs::write(target.join("config.yml"), "modelRoles:\n  default: Editable/FIRST:high\notherSettings:\n  candidates:\n    - editable/first\n    - EDITABLE/*\n").unwrap();
    let blocked = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();
    assert_eq!(blocked.code, "model-delete-unmanaged-reference");
    assert!(
        blocked
            .message
            .contains("config.yml:otherSettings[\"candidates\"][0]")
    );

    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let last = service
        .delete_model(DeleteModelInput {
            opened_models_hash: opened_models_hash(&service),
            opened_config_hash: service
                .get_overview_load()
                .overview
                .unwrap()
                .files
                .config
                .content_hash
                .unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();
    assert_eq!(last.code, "model-last-definition");
}
#[test]
fn model_delete_treats_provider_wildcard_before_literal_model_id() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: "*"
        name: Wildcard ID
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles: {}\nretry:\n  fallback: editable/*\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "first".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "model-delete-unmanaged-reference");
    assert!(error.message.contains("config.yml:retry[\"fallback\"]"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}

#[test]
fn model_delete_rechecks_config_hash_before_atomic_replacement() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = b"providers:\n  editable:\n    baseUrl: https://example.com/v1\n    api: openai-responses\n    models:\n      - id: first\n        name: First\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n      - id: second\n        name: Second\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n";
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();
    service
        .set_models_write_failure_for_test(ModelsWriteFailurePoint::MutateConfigBeforeReplacement);

    let error = service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "second".to_owned(),
        })
        .unwrap_err();
    assert_eq!(error.code, "config-hash-conflict");
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models
    );
    assert!(
        String::from_utf8(fs::read(target.join("config.yml")).unwrap())
            .unwrap()
            .contains("editable/second")
    );
}
#[test]
fn deletion_reference_scan_classifies_roles_and_other_paths_across_both_trees() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"unrecognized:
  selector: editable/second
  unrelated: https://editable/second
providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        r#"modelRoles:
  default: EDITABLE/SECOND:high
  ambiguous: editable/second:ultra
  tagged: !selector editable/second
otherSettings:
  fallback: editable/second
  candidates:
    - editable/*
  comma: editable/first,editable/second
  unrelated: editable/not-the-model
"#,
    )
    .unwrap();

    let dto = serde_json::to_value(
        service_for_target(&target)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();
    let second = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "second")
        .unwrap();
    assert_eq!(second["referenceCount"], 7);
    assert_eq!(
        second["roleReferencePaths"],
        serde_json::json!(["config.yml:modelRoles[\"default\"]"])
    );
    assert_eq!(second["otherReferencePaths"].as_array().unwrap().len(), 6);

    assert!(
        second["otherReferencePaths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "config.yml:modelRoles[\"ambiguous\"]")
    );
    assert!(
        second["otherReferencePaths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "config.yml:otherSettings[\"comma\"]")
    );
    assert!(
        second["otherReferencePaths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "config.yml:modelRoles[\"tagged\"]")
    );
    assert!(
        second["otherReferencePaths"]
            .as_array()
            .unwrap()
            .iter()
            .any(|path| path == "models.yml:unrecognized[\"selector\"]")
    );
    assert!(
        !second["referencePaths"]
            .to_string()
            .contains("https://editable/second")
    );
    assert!(
        !second["referencePaths"]
            .to_string()
            .contains("not-the-model")
    );
    let provider = dto["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|provider| provider["id"] == "editable")
        .unwrap();
    assert_eq!(
        provider["roleReferencePaths"],
        serde_json::json!(["config.yml:modelRoles[\"default\"]"])
    );
    assert_eq!(provider["otherReferencePaths"].as_array().unwrap().len(), 7);
}

#[test]
fn deletion_reference_scan_treats_invalid_role_keys_as_other_paths() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  42: editable/second\n",
    )
    .unwrap();

    let dto = serde_json::to_value(
        service_for_target(&target)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();
    let second = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "second")
        .unwrap();

    assert_eq!(second["roleReferencePaths"], serde_json::json!([]));
    assert_eq!(
        second["otherReferencePaths"],
        serde_json::json!(["config.yml:modelRoles[\"42\"]"])
    );
}

#[test]
fn model_delete_blocks_reference_under_non_string_yaml_key() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles: {}\nfallbacks:\n  42: editable/second\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "second".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "model-delete-unmanaged-reference");
    assert!(error.message.contains("config.yml:fallbacks[\"42\"]"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}

#[test]
fn provider_delete_blocks_reference_under_non_string_yaml_key() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"fallbacks:
  42: editable/first
providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "provider-delete-unmanaged-reference");
    assert!(error.message.contains("models.yml:fallbacks[\"42\"]"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}

#[test]
fn deletion_reference_scan_prefers_existing_full_model_id() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: "second:ultra"
        name: Full ID
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  exact: editable/second:ultra\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let dto = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    let second = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "second")
        .unwrap();
    assert_eq!(second["referenceCount"], 0);
    let full_id = dto["models"]
        .as_array()
        .unwrap()
        .iter()
        .find(|model| model["id"] == "second:ultra")
        .unwrap();
    assert_eq!(
        full_id["roleReferencePaths"],
        serde_json::json!(["config.yml:modelRoles[\"exact\"]"])
    );

    let overview = service.get_overview_load().overview.unwrap();
    service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "second".to_owned(),
        })
        .unwrap();
}

#[test]
fn model_delete_hands_supported_role_references_to_configuration_transaction() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: editable/second\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_model(DeleteModelInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
            model_id: "second".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "model-delete-role-reference");
    assert!(error.message.contains("config.yml:modelRoles[\"default\"]"));
    assert!(error.action.contains("Configuration transaction"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}

#[test]
fn provider_delete_removes_only_the_provider_node_when_reference_free() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"unrecognizedRoot:
  keep: value
providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
  sibling:
    baseUrl: https://sibling.example/v1
    api: openai-responses
    models:
      - id: sibling
        name: Sibling
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let result = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
        })
        .unwrap();

    assert_eq!(result.provider_id, "editable");
    let after: serde_yaml::Value =
        serde_yaml::from_slice(&fs::read(target.join("models.yml")).unwrap()).unwrap();
    assert_eq!(after["unrecognizedRoot"]["keep"], "value");
    assert!(after["providers"]["editable"].is_null());
    assert_eq!(after["providers"]["sibling"]["models"][0]["id"], "sibling");
    assert_eq!(
        fs::read_to_string(target.join("config.yml")).unwrap(),
        "modelRoles: {}\n"
    );
}

#[test]
fn provider_delete_blocks_model_reference_in_other_configuration_path() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles: {}\nretry:\n  fallback: editable/*\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "provider-delete-unmanaged-reference");
    assert!(error.message.contains("config.yml:retry[\"fallback\"]"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
    assert!(error.action.contains("非受管配置"));
}
#[test]
fn provider_delete_hands_supported_role_references_to_configuration_transaction() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: second
        name: Second
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: editable/first\n",
    )
    .unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "provider-delete-role-reference");
    assert!(error.message.contains("config.yml:modelRoles[\"default\"]"));
    assert!(error.action.contains("Configuration transaction"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}
#[test]
fn provider_delete_rejects_provider_with_read_only_model_without_bypass() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original_models = r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: advanced-model
        name: Advanced model
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
        baseUrl: https://model.example/v1
      - id: ordinary
        name: Ordinary
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original_models).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "editable".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "provider-delete-unavailable");
    assert!(error.message.contains("advanced-model"));
    assert!(error.action.contains("绕过只读边界"));
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original_models.as_bytes()
    );
}

#[test]
fn provider_delete_rejects_advanced_provider_without_bypass() {
    let app_data = tempdir().unwrap();
    let target = app_data.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    let original = r#"providers:
  advanced:
    baseUrl: https://example.com/v1
    api: openai-responses
    headers:
      x-test: value
    models:
      - id: first
        name: First
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
"#;
    fs::write(target.join("models.yml"), original).unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = provider_mutation_service(&target, app_data.path());
    let overview = service.get_overview_load().overview.unwrap();

    let error = service
        .delete_provider(DeleteProviderInput {
            opened_models_hash: overview.files.models.content_hash.unwrap(),
            opened_config_hash: overview.files.config.content_hash.unwrap(),
            provider_id: "advanced".to_owned(),
        })
        .unwrap_err();

    assert_eq!(error.code, "provider-delete-unavailable");
    assert_eq!(
        fs::read(target.join("models.yml")).unwrap(),
        original.as_bytes()
    );
}
#[test]
fn overview_marks_malformed_model_fields_read_only() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        r#"providers:
  editable:
    baseUrl: https://example.com/v1
    api: openai-responses
    models:
      - id: malformed-name
        name: 123
        input: [text]
        contextWindow: 100000
        maxTokens: 1000
      - id: malformed-context
        name: Malformed Context
        input: [text]
        contextWindow: nope
        maxTokens: 1000
"#,
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();

    let dto = serde_json::to_value(
        service_for_target(&target)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();
    for id in ["malformed-name", "malformed-context"] {
        let model = dto["models"]
            .as_array()
            .unwrap()
            .iter()
            .find(|model| model["id"] == id)
            .unwrap();
        assert_eq!(model["status"], "read-only", "{id}");
        assert_eq!(model["editable"], false, "{id}");
        assert!(
            model["readOnlyReason"]
                .as_str()
                .unwrap()
                .contains("字段格式不受支持"),
            "{id}: {:?}",
            model["readOnlyReason"]
        );
    }
}

#[test]
fn overview_marks_role_with_unsupported_protocol() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  custom:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: unsupported\n        name: Unsupported\n        api: unsupported-protocol\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/unsupported\n",
    )
    .unwrap();

    let dto = serde_json::to_value(
        service_for_target(&target)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();
    assert_eq!(dto["roles"][0]["status"], "unsupported");
    assert_eq!(dto["roles"][0]["selector"], "custom/unsupported");
    assert_eq!(dto["rolesEditable"], true);
    assert_eq!(dto["rolesAssignable"], false);
}
#[test]
fn overview_locks_unknown_thinking_suffix_on_unsupported_model() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  custom:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: unsupported\n        name: Unsupported\n        api: unsupported-protocol\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: custom/unsupported:ultra\n",
    )
    .unwrap();

    let dto = serde_json::to_value(
        service_for_target(&target)
            .get_overview_load()
            .overview
            .unwrap(),
    )
    .unwrap();
    assert_eq!(dto["roles"][0]["status"], "advanced");
    assert!(dto["roles"][0]["selector"].is_null());
    assert_eq!(dto["rolesEditable"], false);
}

#[test]
fn model_roles_save_sets_and_clears_one_key_without_touching_unknown_config() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: gpt-5.6-luna\n        name: Luna\n        reasoning: true\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: dnslin/gpt-5.6-luna\n  untouched: keep/provider\nsettings:\n  keep: true\n",
    )
    .unwrap();
    let service = service_for_target_with_app_data(&target, &app_data);
    let overview = service.get_overview_load().overview.unwrap();
    let input =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "set",
                "roleId": "default",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "max"
            }]
        }))
        .unwrap();
    service.save_model_roles(input).unwrap();
    let backup_directory = app_data
        .join("target-configuration-backups")
        .join(crate::models_write::content_hash(
            target.canonicalize().unwrap().to_string_lossy().as_bytes(),
        ))
        .join("config.yml");
    assert_eq!(fs::read_dir(backup_directory).unwrap().count(), 1);
    let changed = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(changed.contains("default: dnslin/gpt-5.6-luna:max"));
    assert!(changed.contains("untouched: keep/provider"));
    assert!(changed.contains("keep: true"));
    let refreshed = service.get_overview_load().overview.unwrap();
    let clear =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": refreshed.files.config.content_hash.unwrap(),
            "changes": [{"kind": "clear", "roleId": "default"}]
        }))
        .unwrap();
    service.save_model_roles(clear).unwrap();
    let cleared = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(!cleared.contains("default:"));
    assert!(cleared.contains("untouched: keep/provider"));
    assert!(cleared.contains("keep: true"));
}

#[test]
fn model_roles_revalidate_models_before_commit() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: gpt-5.6-luna\n        name: Luna\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    let original_config = "modelRoles:\n  default: dnslin/gpt-5.6-luna\n";
    fs::write(target.join("config.yml"), original_config).unwrap();
    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let input =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "set",
                "roleId": "default",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "high"
            }]
        }))
        .unwrap();
    service
        .set_models_write_failure_for_test(ModelsWriteFailurePoint::MutateConfigBeforeReplacement);

    let error = service.save_model_roles(input).unwrap_err();
    assert_eq!(error.code, "role-model-hash-conflict");
    assert_eq!(
        fs::read_to_string(target.join("config.yml")).unwrap(),
        original_config
    );
}

#[test]
fn model_roles_accept_case_insensitive_provider_and_model_ids() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  Dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: GPT-5.6-Luna\n        name: Luna\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles:\n  default: Dnslin/GPT-5.6-Luna\nsettings:\n  keep: true\n",
    )
    .unwrap();
    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let input =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "set",
                "roleId": "default",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "high"
            }]
        }))
        .unwrap();
    service.save_model_roles(input).unwrap();
    let saved = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(saved.contains("default: dnslin/gpt-5.6-luna:high"));
    assert!(saved.contains("keep: true"));
}
#[test]
fn model_roles_reject_model_ids_with_selector_delimiters() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: gpt,5\n        name: Comma\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    let original_config = "modelRoles: {}\nsettings:\n  keep: true\n";
    fs::write(target.join("config.yml"), original_config).unwrap();
    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let overview_json = serde_json::to_value(&overview).unwrap();
    assert_eq!(overview_json["rolesAssignable"], false);
    let input =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "create",
                "roleId": "researcher",
                "providerId": "dnslin",
                "modelId": "gpt,5"
            }]
        }))
        .unwrap();
    let error = service.save_model_roles(input).unwrap_err();
    assert_eq!(error.code, "role-selector-invalid");
    assert_eq!(
        fs::read_to_string(target.join("config.yml")).unwrap(),
        original_config
    );
}

#[test]
fn advanced_model_role_configuration_blocks_all_role_writes() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: gpt-5.6-luna\n        name: Luna\n        reasoning: true\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    let original = "modelRoles:\n  advanced:\n    - dnslin/gpt-5.6-luna\n  default: dnslin/gpt-5.6-luna\nsettings:\n  keep: true\n";
    fs::write(target.join("config.yml"), original).unwrap();
    let service = service_for_target(&target);
    let overview = service.get_overview_load().overview.unwrap();
    let input =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "set",
                "roleId": "default",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "max"
            }]
        }))
        .unwrap();
    let error = service.save_model_roles(input).unwrap_err();
    assert_eq!(error.code, "role-advanced-read-only");
    assert_eq!(
        fs::read_to_string(target.join("config.yml")).unwrap(),
        original
    );
}

#[test]
fn custom_model_roles_support_create_rename_edit_and_delete_without_empty_values() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  dnslin:\n    baseUrl: https://example.com\n    api: openai-responses\n    models:\n      - id: gpt-5.6-luna\n        name: Luna\n        reasoning: true\n        input: [text]\n        contextWindow: 100000\n        maxTokens: 1000\n",
    )
    .unwrap();
    fs::write(
        target.join("config.yml"),
        "modelRoles: {}\nsettings:\n  keep: true\n",
    )
    .unwrap();
    let service = service_for_target(&target);
    let invalid_overview = service.get_overview_load().overview.unwrap();
    let invalid =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": invalid_overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "create",
                "roleId": " analyst ",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna"
            }]
        }))
        .unwrap();
    let invalid_error = service.save_model_roles(invalid).unwrap_err();
    assert_eq!(invalid_error.code, "role-id-invalid");
    assert_eq!(
        fs::read_to_string(target.join("config.yml")).unwrap(),
        "modelRoles: {}\nsettings:\n  keep: true\n"
    );

    let overview = service.get_overview_load().overview.unwrap();
    let create =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "create",
                "roleId": "researcher",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "high"
            }]
        }))
        .unwrap();
    service.save_model_roles(create).unwrap();
    let created = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(created.contains("researcher: dnslin/gpt-5.6-luna:high"));
    assert!(created.contains("keep: true"));

    let overview = service.get_overview_load().overview.unwrap();
    let rename =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "rename",
                "roleId": "researcher",
                "newRoleId": "researcher-v2",
                "providerId": "dnslin",
                "modelId": "gpt-5.6-luna",
                "thinkingLevel": "auto"
            }]
        }))
        .unwrap();
    service.save_model_roles(rename).unwrap();
    let renamed = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(!renamed.contains("researcher:"));
    assert!(renamed.contains("researcher-v2: dnslin/gpt-5.6-luna:auto"));

    let overview = service.get_overview_load().overview.unwrap();
    let delete =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{"kind": "delete", "roleId": "researcher-v2"}]
        }))
        .unwrap();
    service.save_model_roles(delete).unwrap();
    let deleted = fs::read_to_string(target.join("config.yml")).unwrap();
    assert!(!deleted.contains("researcher-v2:"));
    assert!(deleted.contains("modelRoles: {}"));
    assert!(deleted.contains("keep: true"));
}

#[tokio::test]
async fn model_test_reloads_saved_openai_completions_and_uses_a_minimal_authenticated_request() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let mut stream = accept_model_test_connection(&listener);
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut request = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let read = stream.read(&mut buffer).unwrap();
            assert!(read > 0);
            request.extend_from_slice(&buffer[..read]);
            if request.windows(4).any(|window| window == b"\r\n\r\n") {
                let header_end = request
                    .windows(4)
                    .position(|window| window == b"\r\n\r\n")
                    .unwrap()
                    + 4;
                let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                let content_length = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length:"))
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= header_end + content_length {
                    break;
                }
            }
        }
        let request_text = String::from_utf8_lossy(&request);
        assert!(request_text.starts_with("POST /v1/chat/completions HTTP/1.1"));
        assert!(request_text.contains("authorization: Bearer saved-secret"));
        let header_end = request
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .unwrap()
            + 4;
        let body: serde_json::Value = serde_json::from_slice(&request[header_end..]).unwrap();
        assert_eq!(body["model"], "test-model");
        assert_eq!(body["max_tokens"], 1);
        assert_eq!(body["messages"][0]["role"], "user");
        assert_eq!(body["messages"][0]["content"], "OMP Switch model test");
        let response = r#"{"choices":[{"message":{"content":"OK"}}]}"#;
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response,
        )
        .unwrap();
    });

    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-provider:\n    baseUrl: http://{address}/v1\n    api: openai-completions\n    apiKey: saved-secret\n    models:\n      - id: test-model\n        name: Test Model\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);

    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-provider",
        "modelId": "test-model"
    }))
    .unwrap();
    let blocked = service.test_model(input.clone()).await.unwrap_err();
    assert_eq!(blocked.code, "model-test-cost-notice-required");
    service.accept_model_test_cost_notice().unwrap();
    let result = service.test_model(input.clone()).await.unwrap();

    server.join().unwrap();
    assert!(result.success);
    assert_eq!(result.provider_id, "test-provider");
    assert_eq!(result.model_id, "test-model");
    assert_eq!(result.protocol.as_str(), "openai-completions");
    assert_eq!(result.status, Some(200));
    assert!(
        !serde_json::to_string(&result)
            .unwrap()
            .contains("saved-secret")
    );
    let overview = service.get_overview_load().overview.unwrap();
    let save_roles =
        serde_json::from_value::<crate::application::SaveModelRolesInput>(serde_json::json!({
            "openedConfigHash": overview.files.config.content_hash.unwrap(),
            "changes": [{
                "kind": "create",
                "roleId": "researcher",
                "providerId": "test-provider",
                "modelId": "test-model"
            }]
        }))
        .unwrap();
    service.save_model_roles(save_roles).unwrap();
    assert!(service.get_model_test_state().result.is_some());
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    let rejected = service.test_model(input.clone()).await.unwrap_err();
    assert_eq!(rejected.code, "model-test-not-eligible");
    assert!(service.get_model_test_state().result.is_none());
    fs::write(target.join("models.yml"), "providers: [\n").unwrap();
    let failed_reload = service.get_overview_load();
    assert_eq!(
        failed_reload.error.as_ref().map(|error| error.code),
        Some("overview-parse-error")
    );
    assert!(service.get_model_test_state().result.is_none());
}
#[tokio::test]
async fn model_test_rejects_an_image_only_model_for_text_probe() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        "providers:\n  image-provider:\n    baseUrl: http://127.0.0.1:1/v1\n    api: openai-responses\n    models:\n      - id: image-only\n        name: Image Only\n        input: [image]\n        contextWindow: 128000\n        maxTokens: 4096\n",
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    let overview = serde_json::to_value(service.get_overview_load().overview.unwrap()).unwrap();
    assert_eq!(overview["providers"][0]["models"][0]["complete"], true);
    service.accept_model_test_cost_notice().unwrap();

    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "image-provider",
        "modelId": "image-only"
    }))
    .unwrap();
    let error = service.test_model(input).await.unwrap_err();

    assert_eq!(error.code, "model-test-not-eligible");
}

#[cfg(unix)]
#[tokio::test]
async fn model_test_rejects_a_non_file_models_path_quickly() {
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    let fifo = target.join("models.fifo");
    let fifo_path = CString::new(fifo.to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo_path.as_ptr(), 0o600) }, 0);

    let mut target_override = writable_target(&target);
    target_override.models.canonical_path = fifo.to_string_lossy().into_owned();
    target_override.models.resolved_path = Some(fifo.to_string_lossy().into_owned());
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        target_override: Some(target_override),
        transaction_root: target.parent().unwrap().join(".app-transactions"),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment, None);
    service.accept_model_test_cost_notice().unwrap();
    service.set_model_test_timeout_for_test(Duration::from_millis(50));
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "blocked-provider",
        "modelId": "blocked-model"
    }))
    .unwrap();

    let started = Instant::now();
    let outcome = tokio::time::timeout(Duration::from_millis(300), service.test_model(input)).await;
    let error = outcome
        .expect("model test should reject non-file paths before the deadline")
        .unwrap_err();
    assert_eq!(error.code, "overview-read-failed");
    assert!(started.elapsed() < Duration::from_millis(250));
}

struct CapturedModelTestRequest {
    method: String,
    target: String,
    headers: HashMap<String, String>,
    body: serde_json::Value,
}

fn accept_model_test_connection(listener: &TcpListener) -> TcpStream {
    listener.set_nonblocking(true).unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false).unwrap();
                return stream;
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => panic!("mock server accept failed: {error}"),
        }
    }
}

fn start_model_test_server(
    request_count: usize,
) -> (String, Receiver<CapturedModelTestRequest>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        for _ in 0..request_count {
            let mut stream = accept_model_test_connection(&listener);
            let request = read_model_test_request(&mut stream);
            let response = if request.target.starts_with("/v1/chat/completions") {
                r#"{"choices":[{"message":{"content":"OK"}}]}"#
            } else if request.target.starts_with("/v1/responses") {
                r#"{"output":[{"type":"message"}]}"#
            } else if request.target.contains("/messages") {
                r#"{"content":[{"type":"text","text":"OK"}]}"#
            } else {
                r#"{"candidates":[{"content":{"parts":[{"text":"OK"}]}}]}"#
            };
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response.len(),
                response,
            )
            .unwrap();
            sender.send(request).unwrap();
        }
    });
    (address, receiver, handle)
}

fn read_model_test_request(stream: &mut TcpStream) -> CapturedModelTestRequest {
    stream
        .set_read_timeout(Some(Duration::from_millis(50)))
        .unwrap();
    let mut request = Vec::new();
    let mut buffer = [0_u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let read = match stream.read(&mut buffer) {
            Ok(read) => read,
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                assert!(
                    Instant::now() < deadline,
                    "timed out while reading model-test request"
                );
                continue;
            }
            Err(error) => panic!("failed to read model-test request: {error}"),
        };
        assert!(read > 0);
        request.extend_from_slice(&buffer[..read]);
        let Some(header_end) = request.windows(4).position(|window| window == b"\r\n\r\n") else {
            continue;
        };
        let header_end = header_end + 4;
        let headers_text = String::from_utf8_lossy(&request[..header_end]);
        let content_length = headers_text
            .lines()
            .find_map(|line| {
                let lower = line.to_ascii_lowercase();
                lower
                    .strip_prefix("content-length:")
                    .and_then(|value| value.trim().parse::<usize>().ok())
            })
            .unwrap_or(0);
        if request.len() < header_end + content_length {
            continue;
        }
        let mut lines = headers_text.lines();
        let request_line = lines.next().unwrap();
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts.next().unwrap().to_owned();
        let target = request_parts.next().unwrap().to_owned();
        let headers = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(key, value)| (key.to_ascii_lowercase(), value.trim().to_owned()))
            .collect();
        let body =
            serde_json::from_slice(&request[header_end..header_end + content_length]).unwrap();
        return CapturedModelTestRequest {
            method,
            target,
            headers,
            body,
        };
    }
}

#[tokio::test]
async fn model_test_uses_protocol_specific_urls_bodies_and_authentication() {
    let (base_url, requests, server) = start_model_test_server(4);
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-completions:\n    baseUrl: {base_url}/v1\n    api: openai-completions\n    apiKey: completions-key\n    models:\n      - id: completions-model\n        name: Completions\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n  test-responses:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    apiKey: responses-key\n    models:\n      - id: responses-model\n        name: Responses\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n  test-anthropic:\n    baseUrl: {base_url}/v1/\n    api: anthropic-messages\n    apiKey: anthropic-key\n    models:\n      - id: anthropic-model\n        name: Anthropic\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n  test-google:\n    baseUrl: {base_url}/v1/?region=us&alt=json\n    api: google-generative-ai\n    apiKey: google-key\n    models:\n      - id: google-model\n        name: Google\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();

    for (provider_id, model_id) in [
        ("test-completions", "completions-model"),
        ("test-responses", "responses-model"),
        ("test-anthropic", "anthropic-model"),
        ("test-google", "google-model"),
    ] {
        let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
            "providerId": provider_id,
            "modelId": model_id
        }))
        .unwrap();
        let result = service.test_model(input).await.unwrap();
        assert!(result.success);
    }
    server.join().unwrap();

    let completions = requests.recv().unwrap();
    assert_eq!(completions.method, "POST");
    assert_eq!(completions.target, "/v1/chat/completions");
    assert_eq!(
        completions.headers["authorization"],
        "Bearer completions-key"
    );
    assert_eq!(
        completions.body,
        serde_json::json!({
            "model": "completions-model",
            "messages": [{"role": "user", "content": "OMP Switch model test"}],
            "max_tokens": 1,
        })
    );

    let responses = requests.recv().unwrap();
    assert_eq!(responses.method, "POST");
    assert_eq!(responses.target, "/v1/responses");
    assert_eq!(responses.headers["authorization"], "Bearer responses-key");
    assert_eq!(
        responses.body,
        serde_json::json!({
            "model": "responses-model",
            "input": "OMP Switch model test",
            "max_output_tokens": 1,
        })
    );

    let anthropic = requests.recv().unwrap();
    assert_eq!(anthropic.method, "POST");
    assert_eq!(anthropic.target, "/v1/messages");
    assert_eq!(anthropic.headers["authorization"], "Bearer anthropic-key");
    assert_eq!(anthropic.headers["anthropic-version"], "2023-06-01");
    assert!(!anthropic.headers.contains_key("x-api-key"));
    assert_eq!(
        anthropic.body,
        serde_json::json!({
            "model": "anthropic-model",
            "max_tokens": 1,
            "messages": [{"role": "user", "content": "OMP Switch model test"}],
        })
    );

    let google = requests.recv().unwrap();
    assert_eq!(google.method, "POST");
    assert!(
        google
            .target
            .starts_with("/v1/models/google-model:streamGenerateContent?region=us&alt=sse")
    );
    assert_eq!(google.headers["x-goog-api-key"], "google-key");
    assert!(!google.headers.contains_key("authorization"));
    assert_eq!(
        google.body,
        serde_json::json!({
            "contents": [{"role": "user", "parts": [{"text": "OMP Switch model test"}]}],
            "generationConfig": {"maxOutputTokens": 1},
        })
    );
}

#[tokio::test]
async fn model_test_never_forwards_credentials_across_redirects() {
    let (base_url, forwarded, server) = start_redirect_model_test_server();
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-redirect:\n    baseUrl: {base_url}\n    api: anthropic-messages\n    apiKey: redirect-secret\n    models:\n      - id: redirect-model\n        name: Redirect\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();

    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-redirect",
        "modelId": "redirect-model"
    }))
    .unwrap();
    let result = service.test_model(input).await.unwrap();

    assert!(!result.success);
    assert_eq!(result.error_code.as_deref(), Some("http-status"));
    assert!(!forwarded.load(Ordering::SeqCst));
    server.join().unwrap();
}

#[tokio::test]
async fn model_test_limits_response_body_and_returns_safe_format_failure() {
    let (base_url, server) = start_oversized_model_test_server();
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-large:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    models:\n      - id: large-model\n        name: Large\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();

    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-large",
        "modelId": "large-model"
    }))
    .unwrap();
    let result = service.test_model(input).await.unwrap();

    assert!(!result.success);
    assert_eq!(result.error_code.as_deref(), Some("response-format"));
    server.join().unwrap();
}

#[tokio::test]
async fn model_test_classifies_base_dns_connection_and_tls_failures_safely() {
    let invalid = ModelTestConfiguration {
        provider_id: "provider".to_owned(),
        model_id: "model".to_owned(),
        base_url: "not-a-url".to_owned(),
        protocol: SupportedApi::OpenAiResponses,
        auth_mode: OverviewAuthMode::None,
        api_key: None,
        target_path: "/tmp/test-target".to_owned(),
        models_hash: "test-models-hash".to_owned(),
    };
    let invalid_error =
        crate::model_test::execute(invalid, CancellationToken::new(), Duration::from_secs(2))
            .await
            .unwrap_err();
    assert_eq!(invalid_error.code, "model-test-base-url");

    let dns = ModelTestConfiguration {
        base_url: "http://omp-switch.invalid/v1".to_owned(),
        ..direct_model_test_configuration()
    };
    let dns_result =
        crate::model_test::execute(dns, CancellationToken::new(), Duration::from_secs(2))
            .await
            .unwrap();
    assert_eq!(dns_result.error_code.as_deref(), Some("dns"));

    let unused_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let unused_address = unused_listener.local_addr().unwrap();
    drop(unused_listener);
    let connection = ModelTestConfiguration {
        base_url: format!("http://{unused_address}/tls-provider/v1"),
        ..direct_model_test_configuration()
    };
    let connection_result =
        crate::model_test::execute(connection, CancellationToken::new(), Duration::from_secs(2))
            .await
            .unwrap();
    assert_eq!(connection_result.error_code.as_deref(), Some("connection"));

    let tls_listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let tls_address = tls_listener.local_addr().unwrap();
    let tls_server = thread::spawn(move || {
        let mut stream = accept_model_test_connection(&tls_listener);
        stream.set_nonblocking(false).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        let mut buffer = [0_u8; 1024];
        let _ = stream.read(&mut buffer);
        let _ = stream.write_all(b"not tls");
    });
    let tls = ModelTestConfiguration {
        base_url: format!("https://{tls_address}/v1"),
        ..direct_model_test_configuration()
    };
    let tls_result =
        crate::model_test::execute(tls, CancellationToken::new(), Duration::from_secs(2))
            .await
            .unwrap();
    assert_eq!(tls_result.error_code.as_deref(), Some("tls"));
    tls_server.join().unwrap();
}
fn direct_model_test_configuration() -> ModelTestConfiguration {
    ModelTestConfiguration {
        provider_id: "provider".to_owned(),
        model_id: "model".to_owned(),
        base_url: "http://127.0.0.1:1/v1".to_owned(),
        protocol: SupportedApi::OpenAiResponses,
        auth_mode: OverviewAuthMode::None,
        api_key: None,
        target_path: "/tmp/test-target".to_owned(),
        models_hash: "test-models-hash".to_owned(),
    }
}

fn start_redirect_model_test_server() -> (String, Arc<AtomicBool>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let destination = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let destination_url = format!("http://{}/v1/messages", destination.local_addr().unwrap());
    let forwarded = Arc::new(AtomicBool::new(false));
    let forwarded_in_server = Arc::clone(&forwarded);
    let handle = thread::spawn(move || {
        let mut stream = accept_model_test_connection(&listener);
        let _ = read_model_test_request(&mut stream);
        write!(
            stream,
            "HTTP/1.1 307 Temporary Redirect\r\nLocation: {destination_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        thread::sleep(Duration::from_millis(100));
        destination.set_nonblocking(true).unwrap();
        if let Ok((mut redirected_stream, _)) = destination.accept() {
            forwarded_in_server.store(true, Ordering::SeqCst);
            let _ = read_model_test_request(&mut redirected_stream);
            let body = r#"{"content":[{"type":"text","text":"OK"}]}"#;
            let _ = write!(
                redirected_stream,
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body,
            );
        }
    });
    (address, forwarded, handle)
}

fn start_oversized_model_test_server() -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let body = "x".repeat(crate::model_test::MAX_RESPONSE_BYTES + 1);
    let handle = thread::spawn(move || {
        let mut stream = accept_model_test_connection(&listener);
        let _ = read_model_test_request(&mut stream);
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body,
        );
    });
    (address, handle)
}

fn start_status_model_test_server(responses: Vec<(u16, &'static str)>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        for (status, response) in responses {
            let mut stream = accept_model_test_connection(&listener);
            let _ = read_model_test_request(&mut stream);
            write!(
                stream,
                "HTTP/1.1 {} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                status,
                response.len(),
                response,
            )
            .unwrap();
        }
    });
    (address, handle)
}

fn start_hanging_model_test_server() -> (String, Receiver<()>, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let address = format!("http://{}", listener.local_addr().unwrap());
    let (sender, receiver) = mpsc::channel();
    let handle = thread::spawn(move || {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let (mut stream, _) = loop {
            match listener.accept() {
                Ok(connection) => break connection,
                Err(error)
                    if error.kind() == std::io::ErrorKind::WouldBlock
                        && std::time::Instant::now() < deadline =>
                {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => panic!("mock server accept failed: {error}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let _ = read_model_test_request(&mut stream);
        sender.send(()).unwrap();
        thread::sleep(Duration::from_millis(400));
        let response = r#"{"output":[{"type":"message"}]}"#;
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response.len(),
            response,
        );
    });
    (address, receiver, handle)
}

#[tokio::test]
async fn model_test_returns_safe_error_categories_and_supports_no_authentication() {
    let (base_url, server) = start_status_model_test_server(vec![
        (401, r#"{"error":"auth response secret"}"#),
        (403, r#"{"error":"permission response secret"}"#),
        (404, r#"{"error":"not found response secret"}"#),
        (429, r#"{"error":"rate response secret"}"#),
        (500, r#"{"error":"server response secret"}"#),
        (200, "not-json response secret"),
    ]);
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-errors:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    apiKey: error-key\n    models:\n      - id: error-model\n        name: Error Model\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();
    for expected in [
        "http-401",
        "http-403",
        "http-404",
        "http-429",
        "http-5xx",
        "response-format",
    ] {
        let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
            "providerId": "test-errors",
            "modelId": "error-model"
        }))
        .unwrap();
        let result = service.test_model(input).await.unwrap();
        assert!(!result.success);
        assert_eq!(result.error_code.as_deref(), Some(expected));
        assert!(
            !serde_json::to_string(&result)
                .unwrap()
                .contains("response secret")
        );
    }
    server.join().unwrap();

    let (base_url, requests, server) = start_model_test_server(1);
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-no-auth:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    models:\n      - id: no-auth-model\n        name: No Auth\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-no-auth",
        "modelId": "no-auth-model"
    }))
    .unwrap();
    assert!(service.test_model(input).await.unwrap().success);
    let request = requests.recv().unwrap();
    assert!(!request.headers.contains_key("authorization"));
    assert!(!request.headers.contains_key("x-api-key"));
    server.join().unwrap();
}

#[tokio::test]
async fn model_test_is_single_concurrent_cancellable_and_time_bounded() {
    let (base_url, started, server) = start_hanging_model_test_server();
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-timeout:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    apiKey: timeout-key\n    models:\n      - id: timeout-model\n        name: Timeout\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();
    service.set_model_test_timeout_for_test(Duration::from_millis(50));
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-timeout",
        "modelId": "timeout-model"
    }))
    .unwrap();
    let result = service.test_model(input).await.unwrap();
    assert_eq!(result.error_code.as_deref(), Some("timeout"));
    started.recv_timeout(Duration::from_secs(1)).unwrap();
    server.join().unwrap();

    let (base_url, started, server) = start_hanging_model_test_server();
    let app_data = tempdir().unwrap().keep();
    let target = app_data.join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(
        target.join("models.yml"),
        format!(
            "providers:\n  test-cancel:\n    baseUrl: {base_url}/v1\n    api: openai-responses\n    apiKey: cancel-key\n    models:\n      - id: cancel-model\n        name: Cancel\n        input: [text]\n        contextWindow: 128000\n        maxTokens: 4096\n"
        ),
    )
    .unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let service = service_for_target(&target);
    service.accept_model_test_cost_notice().unwrap();
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "test-cancel",
        "modelId": "cancel-model"
    }))
    .unwrap();
    let first_service = service.clone();
    let first_input = input.clone();
    let first = tokio::spawn(async move { first_service.test_model(first_input).await.unwrap() });
    let mut request_started = false;
    for _ in 0..100 {
        if started.try_recv().is_ok() {
            request_started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(request_started);
    let busy = service.test_model(input).await.unwrap_err();
    assert_eq!(busy.code, "model-test-busy");
    assert!(service.cancel_model_test());
    let cancelled = first.await.unwrap();
    assert_eq!(cancelled.error_code.as_deref(), Some("cancelled"));
    assert!(!service.get_model_test_state().running);
    server.join().unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn model_test_cancels_and_times_out_a_hanging_omp_preflight() {
    let app_data = tempdir().unwrap().keep();
    let executable = app_data.join("hanging-omp");
    fs::write(&executable, "#!/bin/sh\nwhile :; do :; done\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let settings_path = app_data.join("settings.json");
    fs::write(
        &settings_path,
        serde_json::to_vec_pretty(&AppSettings {
            omp_executable_path: Some(executable.to_string_lossy().into_owned()),
            ..AppSettings::default()
        })
        .unwrap(),
    )
    .unwrap();
    let environment = Arc::new(SystemOmpEnvironment::new(
        app_data.join("target-initialization-transactions"),
    ));
    let service = AppService::new_with_environment(settings_path, environment).unwrap();
    service.accept_model_test_cost_notice().unwrap();
    service.set_model_test_timeout_for_test(Duration::from_secs(2));
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "hanging-provider",
        "modelId": "hanging-model"
    }))
    .unwrap();

    let first_service = service.clone();
    let first_input = input.clone();
    let started = Instant::now();
    let first = tokio::spawn(async move { first_service.test_model(first_input).await });
    for _ in 0..100 {
        if service.get_model_test_state().running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(service.cancel_model_test());
    let cancelled = first.await.unwrap().unwrap_err();
    assert_eq!(cancelled.code, "model-test-cancelled");
    assert!(started.elapsed() < Duration::from_secs(1));
    for _ in 0..100 {
        if !service.get_model_test_state().running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let cancelled_state = service.get_model_test_state();
    assert!(!cancelled_state.running);
    assert_eq!(
        cancelled_state
            .terminal
            .as_ref()
            .map(|terminal| terminal.error_code.as_str()),
        Some("cancelled")
    );
    service.set_model_test_timeout_for_test(Duration::from_millis(50));
    let started = Instant::now();
    let timed_out = service.test_model(input).await.unwrap_err();
    assert_eq!(timed_out.code, "model-test-timeout");
    assert!(started.elapsed() < Duration::from_secs(1));
    for _ in 0..100 {
        if !service.get_model_test_state().running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let timed_out_state = service.get_model_test_state();
    assert!(!timed_out_state.running);
    assert_eq!(
        timed_out_state
            .terminal
            .as_ref()
            .map(|terminal| terminal.error_code.as_str()),
        Some("timeout")
    );
}

#[tokio::test]
async fn model_test_keeps_lease_until_blocking_preflight_worker_exits() {
    let root = tempdir().unwrap();
    let target = root.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let release_preflight = Arc::new(Barrier::new(2));
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: root.path().join("transactions"),
        block_first_version: Some((release_preflight.clone(), Arc::new(AtomicBool::new(false)))),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment.clone(), None);
    service.accept_model_test_cost_notice().unwrap();
    service.set_model_test_timeout_for_test(Duration::from_secs(2));
    let input: crate::application::ModelTestInput = serde_json::from_value(serde_json::json!({
        "providerId": "blocked-provider",
        "modelId": "blocked-model"
    }))
    .unwrap();

    let first_service = service.clone();
    let first_input = input.clone();
    let first = tokio::spawn(async move { first_service.test_model(first_input).await });
    for _ in 0..100 {
        if !environment.calls().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!environment.calls().is_empty());
    assert!(service.cancel_model_test());
    let cancelled = first.await.unwrap().unwrap_err();
    assert_eq!(cancelled.code, "model-test-cancelled");

    let busy = service.test_model(input.clone()).await.unwrap_err();
    assert_eq!(busy.code, "model-test-busy");

    release_preflight.wait();
    for _ in 0..100 {
        if !service.get_model_test_state().running {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(!service.get_model_test_state().running);
    let retry = service.test_model(input).await.unwrap_err();
    assert_ne!(retry.code, "model-test-busy");
}

#[tokio::test]
async fn model_test_times_out_while_waiting_for_a_stuck_omp_detection_lock() {
    let root = tempdir().unwrap();
    let target = root.path().join("agent");
    fs::create_dir_all(&target).unwrap();
    fs::write(target.join("models.yml"), "providers: {}\n").unwrap();
    fs::write(target.join("config.yml"), "modelRoles: {}\n").unwrap();
    let release_detection = Arc::new(Barrier::new(2));
    let environment = Arc::new(FakeOmpEnvironment {
        path_omp: Some(PathBuf::from("/bin/temp-omp")),
        config_path: Some(target.clone()),
        inspect_real_target: true,
        transaction_root: root.path().join("transactions"),
        block_first_version: Some((release_detection.clone(), Arc::new(AtomicBool::new(false)))),
        ..FakeOmpEnvironment::default()
    });
    let service = service_with(environment.clone(), None);
    let detection_service = service.clone();
    let detection = thread::spawn(move || detection_service.get_startup_state());
    let mut command_started = false;
    for _ in 0..100 {
        if !environment.calls().is_empty() {
            command_started = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert!(command_started);
    service.accept_model_test_cost_notice().unwrap();
    service.set_model_test_timeout_for_test(Duration::from_millis(50));
    let started = Instant::now();
    let error = service
        .test_model(
            serde_json::from_value(serde_json::json!({
                "providerId": "blocked-provider",
                "modelId": "blocked-model"
            }))
            .unwrap(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.code, "model-test-timeout");
    assert!(started.elapsed() < Duration::from_secs(1));
    release_detection.wait();
    detection.join().unwrap();
}
