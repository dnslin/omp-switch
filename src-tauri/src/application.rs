use std::{
    collections::HashMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use parking_lot::{Condvar, Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tauri_plugin_opener::OpenerExt;

pub(crate) use crate::model_mutation::{
    CreateModelInput, DeleteModelInput, EditModelInput, ModelMutationResult,
};
pub(crate) use crate::provider_mutation::{
    CreateCustomProviderInput, CreateCustomProviderResult, EditCustomProviderInput,
    EditCustomProviderResult, ProviderMutationFailurePoint,
};

#[cfg(test)]
pub(crate) use crate::model_mutation::{ModelDefinitionFields, ModelEditFields};
#[cfg(test)]
pub(crate) use crate::provider_mutation::{
    CreateModelFields, CreateProviderFields, DirectApiKeyIntent, ProviderAuthMode, SupportedApi,
    SupportedInput,
};

use crate::{
    bundled_catalog,
    error::{AppError, io_error_cause},
    model_mutation,
    omp_environment::{OmpEnvironment, SystemOmpEnvironment},
    overview::{OverviewDto, OverviewReadResult, read_overview},
    provider_mutation,
    redaction::redact_diagnostic,
    target_configuration::{
        TargetConfigurationDiscovery, TargetConfigurationStatus, TargetInitializationExpectation,
    },
};

#[cfg(test)]
use crate::overview::ConfigurationSnapshot;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum StartupState {
    OmpUnavailable {
        message: String,
    },
    InvalidExecutable {
        executable_path: String,
        message: String,
        diagnostic_code: String,
    },
    VersionFailed {
        executable_path: String,
        message: String,
        diagnostic_code: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    ConfigPathFailed {
        executable_path: String,
        version: String,
        message: String,
        diagnostic_code: String,
        exit_code: Option<i32>,
        stderr: String,
    },
    OmpReady {
        executable_path: String,
        version: String,
        target_configuration: Box<TargetConfigurationDiscovery>,
        previous_target_configuration: Option<String>,
        requires_confirmation: bool,
    },
}
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OverviewLoadDto {
    pub startup_state: StartupState,
    pub overview: Option<OverviewDto>,
    pub error: Option<AppError>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Theme {
    Light,
    Dark,
    #[default]
    System,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AppSettings {
    pub omp_executable_path: Option<String>,
    pub theme: Theme,
    pub selected_provider_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub cost_notice_accepted: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct UiSettingsUpdate {
    pub theme: Theme,
    pub selected_provider_id: Option<String>,
    pub selected_model_id: Option<String>,
    pub cost_notice_accepted: bool,
}

struct OverviewLoadCoordinator {
    state: Mutex<OverviewLoadState>,
    ready: Condvar,
    #[cfg(test)]
    pause_after_completion: Mutex<Option<OverviewWaiterPause>>,
}

#[derive(Default)]
struct OverviewLoadState {
    next_generation: u64,
    in_flight: Option<u64>,
    completed: HashMap<u64, OverviewLoadDto>,
    waiters: HashMap<u64, usize>,
}

#[cfg(test)]
struct OverviewWaiterPause {
    reached: Arc<std::sync::Barrier>,
    release: Arc<std::sync::Barrier>,
}

impl OverviewLoadCoordinator {
    fn begin_or_wait(&self) -> Option<OverviewLoadDto> {
        let mut state = self.state.lock();
        if let Some(generation) = state.in_flight {
            *state.waiters.entry(generation).or_default() += 1;
            while !state.completed.contains_key(&generation) {
                self.ready.wait(&mut state);
            }

            #[cfg(test)]
            {
                let pause = self.pause_after_completion.lock().take();
                if let Some(pause) = pause {
                    drop(state);
                    pause.reached.wait();
                    pause.release.wait();
                    state = self.state.lock();
                }
            }

            let result = state
                .completed
                .get(&generation)
                .cloned()
                .expect("completed overview flight disappeared");
            let remaining = state
                .waiters
                .get_mut(&generation)
                .expect("overview flight waiter count disappeared");
            *remaining -= 1;
            if *remaining == 0 {
                state.waiters.remove(&generation);
                state.completed.remove(&generation);
            }
            self.ready.notify_all();
            return Some(result);
        }

        state.next_generation = state
            .next_generation
            .checked_add(1)
            .expect("overview load generation exhausted");
        state.in_flight = Some(state.next_generation);
        None
    }

    fn finish(&self, result: &OverviewLoadDto) {
        let mut state = self.state.lock();
        let generation = state
            .in_flight
            .take()
            .expect("overview load finished without an active flight");
        if state.waiters.contains_key(&generation) {
            state.completed.insert(generation, result.clone());
        }
        self.ready.notify_all();
    }

    #[cfg(test)]
    fn waiter_count(&self) -> usize {
        self.state.lock().waiters.values().sum()
    }

    #[cfg(test)]
    fn pause_next_waiter_after_completion(
        &self,
        reached: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        *self.pause_after_completion.lock() = Some(OverviewWaiterPause { reached, release });
    }
}

impl Default for OverviewLoadCoordinator {
    fn default() -> Self {
        Self {
            state: Mutex::new(OverviewLoadState::default()),
            ready: Condvar::new(),
            #[cfg(test)]
            pause_after_completion: Mutex::new(None),
        }
    }
}

#[derive(Clone)]
pub struct AppService {
    settings_path: Arc<PathBuf>,
    backup_root: Arc<PathBuf>,
    settings: Arc<RwLock<AppSettings>>,
    settings_write: Arc<Mutex<()>>,
    environment: Arc<dyn OmpEnvironment>,
    pending_omp: Arc<RwLock<Option<PathBuf>>>,
    recovery_notice: Arc<RwLock<Option<(PathBuf, String)>>>,
    #[cfg(test)]
    configuration_snapshot: Arc<RwLock<Option<ConfigurationSnapshot>>>,
    detection_lock: Arc<Mutex<()>>,
    overview_load: Arc<OverviewLoadCoordinator>,
    #[cfg(test)]
    provider_mutation_failure: Arc<Mutex<Option<ProviderMutationFailurePoint>>>,
}

#[derive(Clone, Copy)]
enum ProviderWriteOperation {
    Create,
    Edit,
}

impl ProviderWriteOperation {
    fn confirmation_required_error(self) -> AppError {
        match self {
            Self::Create => AppError::new(
                "provider-create-confirmation-required",
                "尚未确认新的 OMP 与 Target configuration，不能创建 Provider。",
                "请先在设置页面确认 OMP 切换后重试。",
            ),
            Self::Edit => AppError::new(
                "provider-edit-confirmation-required",
                "尚未确认新的 OMP 与 Target configuration，不能编辑 Provider。",
                "请先在设置页面确认 OMP 切换后重试。",
            ),
        }
    }

    fn unavailable_error(self) -> AppError {
        match self {
            Self::Create => AppError::new(
                "provider-create-unavailable",
                "无法重新验证 OMP 的 Target configuration。",
                "请重新检测或重新选择 OMP。",
            ),
            Self::Edit => AppError::new(
                "provider-edit-unavailable",
                "无法重新验证 OMP 的 Target configuration。",
                "请重新检测或重新选择 OMP。",
            ),
        }
    }

    fn catalog_missing_error(self) -> AppError {
        match self {
            Self::Create => AppError::new(
                "provider-create-catalog-missing",
                "当前 OMP 版本没有匹配的 bundled Provider 清单。",
                "为避免覆盖 OMP 内置 Provider，Provider 与模型管理暂时只读。",
            ),
            Self::Edit => AppError::new(
                "provider-edit-catalog-missing",
                "当前 OMP 版本没有匹配的 bundled Provider 清单。",
                "为避免覆盖 OMP 内置 Provider，Provider 与模型管理暂时只读。",
            ),
        }
    }
}
#[derive(Clone, Copy)]
enum ModelWriteOperation {
    Create,
    Edit,
    Delete,
}

impl ModelWriteOperation {
    fn confirmation_required_error(self) -> AppError {
        let operation = match self {
            Self::Create => "创建模型",
            Self::Edit => "编辑模型",
            Self::Delete => "删除模型",
        };
        AppError::new(
            match self {
                Self::Create => "model-create-confirmation-required",
                Self::Edit => "model-edit-confirmation-required",
                Self::Delete => "model-delete-confirmation-required",
            },
            format!("尚未确认新的 OMP 与 Target configuration，不能{operation}。"),
            "请先在设置页面确认 OMP 与 Target configuration 后重试。",
        )
    }

    fn unavailable_error(self) -> AppError {
        AppError::new(
            match self {
                Self::Create => "model-create-unavailable",
                Self::Edit => "model-edit-unavailable",
                Self::Delete => "model-delete-unavailable",
            },
            "无法重新验证 OMP 的 Target configuration。",
            "请重新检测或重新选择 OMP。",
        )
    }

    fn catalog_missing_error(self) -> AppError {
        AppError::new(
            match self {
                Self::Create => "model-create-catalog-missing",
                Self::Edit => "model-edit-catalog-missing",
                Self::Delete => "model-delete-catalog-missing",
            },
            "当前 OMP 版本没有匹配的 bundled Provider 清单。",
            "为避免覆盖 OMP 内置 Provider，Provider 与模型管理暂时只读。",
        )
    }
}

struct ProviderWriteContext {
    target: Box<TargetConfigurationDiscovery>,
    catalog: &'static bundled_catalog::BundledCatalog,
}
impl AppService {
    pub fn new(settings_path: PathBuf) -> Result<Self, AppError> {
        let transaction_root = settings_path
            .parent()
            .ok_or_else(|| AppError::internal("界面设置路径没有父目录"))?
            .join("target-initialization-transactions");
        Self::new_with_environment(
            settings_path,
            Arc::new(SystemOmpEnvironment::new(transaction_root)),
        )
    }

    pub fn new_with_environment(
        settings_path: PathBuf,
        environment: Arc<dyn OmpEnvironment>,
    ) -> Result<Self, AppError> {
        let backup_root = settings_path
            .parent()
            .ok_or_else(|| AppError::internal("界面设置路径没有父目录"))?
            .join("target-configuration-backups");
        let settings = load_settings(&settings_path)?;
        Ok(Self {
            settings_path: Arc::new(settings_path),
            backup_root: Arc::new(backup_root),
            settings: Arc::new(RwLock::new(settings)),
            settings_write: Arc::new(Mutex::new(())),
            environment,
            pending_omp: Arc::new(RwLock::new(None)),
            recovery_notice: Arc::new(RwLock::new(None)),
            #[cfg(test)]
            configuration_snapshot: Arc::new(RwLock::new(None)),
            detection_lock: Arc::new(Mutex::new(())),
            overview_load: Arc::new(OverviewLoadCoordinator::default()),
            #[cfg(test)]
            provider_mutation_failure: Arc::new(Mutex::new(None)),
        })
    }

    pub fn get_startup_state(&self) -> StartupState {
        let _detection = self.detection_lock.lock();
        self.detect_omp_internal()
    }

    pub fn get_overview_load(&self) -> OverviewLoadDto {
        if let Some(result) = self.overview_load.begin_or_wait() {
            return result;
        }

        let result = {
            let _detection = self.detection_lock.lock();
            let startup_state = self.detect_omp_internal();
            let safe_startup_state = Self::sanitize_overview_startup_state(&startup_state);
            match self.read_overview_for_state(startup_state) {
                Ok(overview) => OverviewLoadDto {
                    startup_state: safe_startup_state,
                    overview: Some(overview),
                    error: None,
                },
                Err(error) => OverviewLoadDto {
                    startup_state: safe_startup_state,
                    overview: None,
                    error: Some(error),
                },
            }
        };
        self.overview_load.finish(&result);
        result
    }

    pub(crate) fn sanitize_overview_startup_state(state: &StartupState) -> StartupState {
        match state {
            StartupState::OmpUnavailable { message } => StartupState::OmpUnavailable {
                message: redact_diagnostic(message),
            },
            StartupState::InvalidExecutable {
                executable_path,
                message,
                diagnostic_code,
            } => StartupState::InvalidExecutable {
                executable_path: executable_path.clone(),
                message: redact_diagnostic(message),
                diagnostic_code: diagnostic_code.clone(),
            },
            StartupState::VersionFailed {
                executable_path,
                message,
                diagnostic_code,
                exit_code,
                stderr,
            } => StartupState::VersionFailed {
                executable_path: executable_path.clone(),
                message: redact_diagnostic(message),
                diagnostic_code: diagnostic_code.clone(),
                exit_code: *exit_code,
                stderr: redact_diagnostic(stderr),
            },
            StartupState::ConfigPathFailed {
                executable_path,
                version,
                message,
                diagnostic_code,
                exit_code,
                stderr,
            } => StartupState::ConfigPathFailed {
                executable_path: executable_path.clone(),
                version: version.clone(),
                message: redact_diagnostic(message),
                diagnostic_code: diagnostic_code.clone(),
                exit_code: *exit_code,
                stderr: redact_diagnostic(stderr),
            },
            StartupState::OmpReady {
                executable_path,
                version,
                target_configuration,
                previous_target_configuration,
                requires_confirmation,
            } => {
                let mut target_configuration = target_configuration.as_ref().clone();
                target_configuration.recovery_notice = target_configuration
                    .recovery_notice
                    .as_deref()
                    .map(redact_diagnostic);
                target_configuration.warnings = target_configuration
                    .warnings
                    .iter()
                    .map(|warning| redact_diagnostic(warning))
                    .collect();
                if let Some(issue) = target_configuration.issue.as_mut() {
                    issue.message = redact_diagnostic(&issue.message);
                }
                StartupState::OmpReady {
                    executable_path: executable_path.clone(),
                    version: version.clone(),
                    target_configuration: Box::new(target_configuration),
                    previous_target_configuration: previous_target_configuration.clone(),
                    requires_confirmation: *requires_confirmation,
                }
            }
        }
    }

    fn read_overview_for_state(&self, state: StartupState) -> Result<OverviewDto, AppError> {
        self.clear_configuration_snapshot();
        let (executable_path, version, target_configuration) = match state {
            StartupState::OmpReady {
                executable_path,
                version,
                target_configuration,
                requires_confirmation: false,
                ..
            } => (executable_path, version, target_configuration),
            StartupState::OmpReady {
                requires_confirmation: true,
                ..
            } => {
                return Err(AppError::new(
                    "overview-confirmation-required",
                    "无法读取尚未确认的 OMP 配置切换。",
                    "请返回“设置 OMP”页确认新的 OMP 与 Target configuration 后再读取概览。",
                ));
            }
            _ => {
                return Err(AppError::new(
                    "overview-omp-unavailable",
                    "无法读取 OMP 概览。",
                    "请先完成 OMP 检测并确认有效的 Target configuration。",
                ));
            }
        };
        let result = read_overview(&executable_path, &version, &target_configuration)?;
        Ok(self.finish_overview_read(result))
    }

    #[cfg(test)]
    fn clear_configuration_snapshot(&self) {
        *self.configuration_snapshot.write() = None;
    }

    #[cfg(not(test))]
    fn clear_configuration_snapshot(&self) {}

    #[cfg(test)]
    fn finish_overview_read(&self, result: OverviewReadResult) -> OverviewDto {
        *self.configuration_snapshot.write() = result.snapshot;
        result.dto
    }

    #[cfg(not(test))]
    fn finish_overview_read(&self, result: OverviewReadResult) -> OverviewDto {
        result.dto
    }

    #[cfg(test)]
    pub(crate) fn configuration_snapshot_for_test(&self) -> Option<ConfigurationSnapshot> {
        self.configuration_snapshot.read().clone()
    }

    #[cfg(test)]
    pub(crate) fn overview_waiters_for_test(&self) -> usize {
        self.overview_load.waiter_count()
    }

    #[cfg(test)]
    pub(crate) fn pause_next_overview_waiter_for_test(
        &self,
        reached: Arc<std::sync::Barrier>,
        release: Arc<std::sync::Barrier>,
    ) {
        self.overview_load
            .pause_next_waiter_after_completion(reached, release);
    }

    pub fn detect_omp(&self) -> StartupState {
        let _detection = self.detection_lock.lock();
        *self.recovery_notice.write() = None;
        self.detect_omp_internal()
    }

    fn detect_omp_internal(&self) -> StartupState {
        *self.pending_omp.write() = None;
        let saved = self.settings.read().omp_executable_path.clone();
        let mut saved_failure = None;
        if let Some(path) = saved.as_ref() {
            let state = self.validate_omp(PathBuf::from(path), false, None);
            if matches!(state, StartupState::OmpReady { .. }) {
                return state;
            }
            saved_failure = Some(state);
        }
        match self.environment.find_in_path() {
            Ok(Some(path)) => {
                let requires_confirmation = saved.is_some();
                let state = self.validate_omp(path.clone(), requires_confirmation, None);
                *self.pending_omp.write() =
                    matches!(state, StartupState::OmpReady { .. } if requires_confirmation)
                        .then_some(path);
                state
            }
            Ok(None) => saved_failure.unwrap_or_else(|| StartupState::OmpUnavailable {
                message: "未在已保存路径或系统 PATH 中找到可用的 OMP。".to_owned(),
            }),
            Err(error) => {
                let cause = io_error_cause(error.kind());
                tracing::warn!(
                    operation = "find_omp_in_path",
                    cause,
                    "OMP PATH discovery failed"
                );
                saved_failure.unwrap_or_else(|| StartupState::OmpUnavailable {
                    message: format!("无法检查系统 PATH 中的 OMP（{cause}）。请手动选择 OMP。"),
                })
            }
        }
    }

    pub fn validate_selected_omp(&self, executable: PathBuf) -> StartupState {
        let _detection = self.detection_lock.lock();
        *self.pending_omp.write() = None;
        let previous_target_configuration = self.saved_target_configuration(&executable);
        let state = self.validate_omp(executable.clone(), true, previous_target_configuration);
        *self.pending_omp.write() =
            matches!(state, StartupState::OmpReady { .. }).then_some(executable);
        state
    }

    fn saved_target_configuration(&self, selected: &Path) -> Option<String> {
        let saved = self.settings.read().omp_executable_path.clone()?;
        if Path::new(&saved) == selected {
            return None;
        }
        match self.validate_omp(PathBuf::from(saved), false, None) {
            StartupState::OmpReady {
                target_configuration,
                ..
            } => Some(target_configuration.path),
            _ => None,
        }
    }

    fn validate_omp(
        &self,
        executable: PathBuf,
        requires_confirmation: bool,
        previous_target_configuration: Option<String>,
    ) -> StartupState {
        let executable_path = executable.to_string_lossy().into_owned();
        let version_output = match self.environment.run(&executable, &["--version"]) {
            Ok(output) => output,
            Err(error) => {
                return StartupState::InvalidExecutable {
                    executable_path,
                    message: "所选文件无法作为 OMP 可执行文件运行。".to_owned(),
                    diagnostic_code: io_error_cause(error.kind()).to_owned(),
                };
            }
        };
        if !version_output.success {
            return StartupState::VersionFailed {
                executable_path,
                message: "OMP 版本命令执行失败。".to_owned(),
                diagnostic_code: "process-exit".to_owned(),
                exit_code: version_output.exit_code,
                stderr: redact_diagnostic(&version_output.stderr),
            };
        }
        let version =
            parse_single_line(&version_output.stdout).unwrap_or_else(|| "未知版本".to_owned());
        let path_output = match self.environment.run(&executable, &["config", "path"]) {
            Ok(output) => output,
            Err(error) => {
                return StartupState::ConfigPathFailed {
                    executable_path,
                    version,
                    message: config_path_failure_message(),
                    diagnostic_code: io_error_cause(error.kind()).to_owned(),
                    exit_code: None,
                    stderr: String::new(),
                };
            }
        };
        let target = parse_absolute_directory(&path_output.stdout);
        if !path_output.success || target.is_none() {
            return StartupState::ConfigPathFailed {
                executable_path,
                version,
                message: config_path_failure_message(),
                diagnostic_code: if path_output.success {
                    "invalid-output"
                } else {
                    "process-exit"
                }
                .to_owned(),
                exit_code: path_output.exit_code,
                stderr: redact_diagnostic(&path_output.stderr),
            };
        }
        let target = target.unwrap();
        let mut target_configuration = match self.environment.inspect_target(&target) {
            Ok(discovery) => discovery,
            Err(error) => {
                return StartupState::ConfigPathFailed {
                    executable_path,
                    version,
                    message: "权威配置目录及其父目录不可访问。OMP Switch 不会改用其他目录。"
                        .to_owned(),
                    diagnostic_code: io_error_cause(error.kind()).to_owned(),
                    exit_code: None,
                    stderr: redact_diagnostic(&error.to_string()),
                };
            }
        };
        if let Some(notice) = target_configuration.recovery_notice.clone() {
            *self.recovery_notice.write() = Some((target.clone(), notice));
        } else if let Some((notice_target, notice)) = self.recovery_notice.read().as_ref()
            && notice_target == &target
        {
            target_configuration.recovery_notice = Some(notice.clone());
        }
        StartupState::OmpReady {
            executable_path,
            version,
            target_configuration: Box::new(target_configuration),
            previous_target_configuration,
            requires_confirmation,
        }
    }

    pub fn initialize_target_configuration(
        &self,
        executable: PathBuf,
        expectation: TargetInitializationExpectation,
    ) -> Result<StartupState, AppError> {
        let _detection = self.detection_lock.lock();
        let saved = self.settings.read().omp_executable_path.clone();
        let requires_confirmation = self.pending_omp.read().as_ref() == Some(&executable)
            || saved
                .as_deref()
                .is_some_and(|saved_path| Path::new(saved_path) != executable);
        let previous_target_configuration = self.saved_target_configuration(&executable);
        let state = self.validate_omp(
            executable.clone(),
            requires_confirmation,
            previous_target_configuration.clone(),
        );
        let target = match state {
            StartupState::OmpReady {
                ref target_configuration,
                ..
            } if target_configuration.status == TargetConfigurationStatus::CreationRequired => {
                if target_configuration.create_paths != expectation.create_paths
                    || target_configuration.discovery_token != expectation.discovery_token
                {
                    return Err(AppError::new(
                        "target-initialization-changed",
                        "Target configuration 在确认后发生变化。",
                        "请重新检测并确认最新的真实目标和创建文件清单。",
                    ));
                }
                PathBuf::from(&target_configuration.path)
            }
            StartupState::OmpReady { .. } => {
                return Err(AppError::new(
                    "target-initialization-not-required",
                    "Target configuration 当前状态不允许创建最小配置。",
                    "请重新检测并按当前只读、迁移或错误提示处理。",
                ));
            }
            _ => {
                return Err(AppError::new(
                    "target-initialization-unavailable",
                    "无法重新验证 OMP 的权威配置目录。",
                    "请重新检测或重新选择 OMP。",
                ));
            }
        };
        if requires_confirmation {
            self.confirm_selected_omp_locked(executable.clone())?;
        }
        if let Err(error) = self.environment.initialize_target(&target, &expectation) {
            let recovery_incomplete = error.recovery_incomplete();
            tracing::warn!(
                operation = "initialize_target_configuration",
                cause = io_error_cause(error.kind()),
                diagnostic = %redact_diagnostic(&error.to_string()),
                recovery_incomplete,
                "Target configuration initialization failed"
            );
            if requires_confirmation && !recovery_incomplete {
                self.update_settings(|settings| {
                    settings.omp_executable_path = saved.clone();
                })?;
                *self.pending_omp.write() = Some(executable.clone());
            }
            return Err(AppError::new(
                "target-initialization-failed",
                if recovery_incomplete {
                    "创建最小 Target configuration 失败，且事务恢复尚未完整完成。"
                } else {
                    "创建最小 Target configuration 失败。"
                },
                if recovery_incomplete {
                    "已保留新的 OMP 选择以便下次启动继续恢复。请勿继续写入，并重新检测。"
                } else {
                    "未保留部分创建结果，原 OMP 选择已恢复。请检查路径、权限和链接状态后重试。"
                },
            ));
        }
        if self
            .recovery_notice
            .read()
            .as_ref()
            .is_some_and(|(notice_target, _)| notice_target == &target)
        {
            *self.recovery_notice.write() = None;
        }
        Ok(self.validate_omp(executable, false, None))
    }

    pub fn target_directory_for_opening(&self, executable: PathBuf) -> Result<PathBuf, AppError> {
        let _detection = self.detection_lock.lock();
        match self.validate_omp(executable, false, None) {
            StartupState::OmpReady {
                target_configuration,
                ..
            } => target_configuration
                .resolved_path
                .map(PathBuf::from)
                .ok_or_else(|| {
                    AppError::new(
                        "target-directory-unresolved",
                        "无法确认 Target configuration 的真实目录。",
                        "请修复链接或路径问题后重新检测。",
                    )
                }),
            _ => Err(AppError::new(
                "target-directory-unavailable",
                "无法重新验证 OMP 的 Target configuration。",
                "请重新检测或重新选择 OMP。",
            )),
        }
    }

    pub fn confirm_selected_omp(&self, executable: PathBuf) -> Result<AppSettings, AppError> {
        let _detection = self.detection_lock.lock();
        self.confirm_selected_omp_locked(executable)
    }

    fn confirm_selected_omp_locked(&self, executable: PathBuf) -> Result<AppSettings, AppError> {
        let mut pending = self.pending_omp.write();
        if pending.as_ref() != Some(&executable) {
            return Err(AppError::internal("OMP 验证状态已变化，请重新检测"));
        }
        let settings = self.update_settings(|settings| {
            settings.omp_executable_path = Some(executable.to_string_lossy().into_owned());
        })?;
        *pending = None;
        Ok(settings)
    }
    pub fn get_ui_settings(&self) -> Result<AppSettings, AppError> {
        Ok(self.settings.read().clone())
    }

    pub fn save_ui_settings(&self, update: UiSettingsUpdate) -> Result<AppSettings, AppError> {
        self.update_settings(|settings| {
            settings.theme = update.theme;
            settings.selected_provider_id = update.selected_provider_id;
            settings.selected_model_id = update.selected_model_id;
            settings.cost_notice_accepted = update.cost_notice_accepted;
        })
    }

    pub fn create_custom_provider(
        &self,
        input: CreateCustomProviderInput,
    ) -> Result<CreateCustomProviderResult, AppError> {
        let _detection = self.detection_lock.lock();
        let context = self.prepare_provider_write(ProviderWriteOperation::Create)?;
        let result = provider_mutation::create_custom_provider(
            &context.target,
            &self.backup_root,
            context.catalog,
            &input,
            self.take_provider_mutation_failure(),
        );
        if result.is_ok() {
            self.clear_configuration_snapshot();
        }
        result
    }

    pub fn edit_custom_provider(
        &self,
        input: EditCustomProviderInput,
    ) -> Result<EditCustomProviderResult, AppError> {
        let _detection = self.detection_lock.lock();
        let context = self.prepare_provider_write(ProviderWriteOperation::Edit)?;
        let result = provider_mutation::edit_custom_provider(
            &context.target,
            &self.backup_root,
            context.catalog,
            &input,
            self.take_provider_mutation_failure(),
        );
        if result.is_ok() {
            self.clear_configuration_snapshot();
        }
        result
    }

    pub fn create_model(&self, input: CreateModelInput) -> Result<ModelMutationResult, AppError> {
        let _detection = self.detection_lock.lock();
        let context = self.prepare_model_write(ModelWriteOperation::Create)?;
        let result = model_mutation::create_model(
            &context.target,
            &self.backup_root,
            context.catalog,
            &input,
            self.take_provider_mutation_failure(),
        );
        if result.is_ok() {
            self.clear_configuration_snapshot();
        }
        result
    }

    pub fn edit_model(&self, input: EditModelInput) -> Result<ModelMutationResult, AppError> {
        let _detection = self.detection_lock.lock();
        let context = self.prepare_model_write(ModelWriteOperation::Edit)?;
        let result = model_mutation::edit_model(
            &context.target,
            &self.backup_root,
            context.catalog,
            &input,
            self.take_provider_mutation_failure(),
        );
        if result.is_ok() {
            self.clear_configuration_snapshot();
        }
        result
    }

    pub fn delete_model(&self, input: DeleteModelInput) -> Result<ModelMutationResult, AppError> {
        let _detection = self.detection_lock.lock();
        let context = self.prepare_model_write(ModelWriteOperation::Delete)?;
        let result = model_mutation::delete_model(
            &context.target,
            &self.backup_root,
            context.catalog,
            &input,
            self.take_provider_mutation_failure(),
        );
        if result.is_ok() {
            self.clear_configuration_snapshot();
        }
        result
    }

    fn prepare_model_write(
        &self,
        operation: ModelWriteOperation,
    ) -> Result<ProviderWriteContext, AppError> {
        let state = self.detect_omp_internal();
        let (version, target) = match state {
            StartupState::OmpReady {
                version,
                target_configuration,
                requires_confirmation: false,
                ..
            } => (version, target_configuration),
            StartupState::OmpReady { .. } => return Err(operation.confirmation_required_error()),
            _ => return Err(operation.unavailable_error()),
        };
        let catalog = bundled_catalog::for_version(&version)?
            .ok_or_else(|| operation.catalog_missing_error())?;
        Ok(ProviderWriteContext { target, catalog })
    }
    fn prepare_provider_write(
        &self,
        operation: ProviderWriteOperation,
    ) -> Result<ProviderWriteContext, AppError> {
        let state = self.detect_omp_internal();
        let (version, target) = match state {
            StartupState::OmpReady {
                version,
                target_configuration,
                requires_confirmation: false,
                ..
            } => (version, target_configuration),
            StartupState::OmpReady { .. } => return Err(operation.confirmation_required_error()),
            _ => return Err(operation.unavailable_error()),
        };
        let catalog = bundled_catalog::for_version(&version)?
            .ok_or_else(|| operation.catalog_missing_error())?;
        Ok(ProviderWriteContext { target, catalog })
    }

    fn take_provider_mutation_failure(&self) -> Option<ProviderMutationFailurePoint> {
        #[cfg(test)]
        {
            self.provider_mutation_failure.lock().take()
        }
        #[cfg(not(test))]
        {
            None
        }
    }

    #[cfg(test)]
    pub(crate) fn set_provider_mutation_failure_for_test(
        &self,
        failure: ProviderMutationFailurePoint,
    ) {
        *self.provider_mutation_failure.lock() = Some(failure);
    }

    fn update_settings(
        &self,
        mutate: impl FnOnce(&mut AppSettings),
    ) -> Result<AppSettings, AppError> {
        let _transaction = self.settings_write.lock();
        let mut settings = self.settings.read().clone();
        mutate(&mut settings);
        persist_settings(&self.settings_path, &settings)?;
        *self.settings.write() = settings.clone();
        Ok(settings)
    }
}

fn parse_single_line(value: &str) -> Option<String> {
    let mut lines = value.lines().map(str::trim).filter(|line| !line.is_empty());
    let value = lines.next()?;
    if lines.next().is_some() {
        None
    } else {
        Some(value.to_owned())
    }
}

fn parse_absolute_directory(value: &str) -> Option<PathBuf> {
    let path = PathBuf::from(parse_single_line(value)?);
    path.is_absolute().then_some(path)
}

fn config_path_failure_message() -> String {
    "OMP 没有成功返回一个绝对配置目录。OMP Switch 不会猜测目录。该命令可能初始化 OMP Settings、访问 agent.db，或运行 OMP 自身的旧迁移。".to_owned()
}

fn load_settings(path: &Path) -> Result<AppSettings, AppError> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| {
            let cause = match error.classify() {
                serde_json::error::Category::Io => "settings-json-io",
                serde_json::error::Category::Syntax => "settings-json-syntax",
                serde_json::error::Category::Data => "settings-json-data",
                serde_json::error::Category::Eof => "settings-json-eof",
            };
            internal_error_with_cause("load_ui_settings", cause, "界面设置文件无法解析")
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(error) => Err(internal_error_with_cause(
            "load_ui_settings",
            io_error_cause(error.kind()),
            "无法读取界面设置文件",
        )),
    }
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), AppError> {
    let parent = path.parent().ok_or_else(|| {
        internal_error_with_cause("persist_ui_settings", "missing-parent", "界面设置路径无效")
    })?;
    fs::create_dir_all(parent).map_err(|error| {
        internal_error_with_cause(
            "persist_ui_settings",
            io_error_cause(error.kind()),
            "无法创建应用数据目录",
        )
    })?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|_| {
        internal_error_with_cause(
            "persist_ui_settings",
            "settings-json-serialize",
            "无法序列化界面设置",
        )
    })?;
    let mut file = atomic_write_file::AtomicWriteFile::options()
        .open(path)
        .map_err(|error| {
            internal_error_with_cause(
                "persist_ui_settings",
                io_error_cause(error.kind()),
                "无法创建界面设置临时文件",
            )
        })?;
    file.write_all(&bytes).map_err(|error| {
        internal_error_with_cause(
            "persist_ui_settings",
            io_error_cause(error.kind()),
            "无法写入界面设置",
        )
    })?;
    file.commit().map_err(|error| {
        internal_error_with_cause(
            "persist_ui_settings",
            io_error_cause(error.kind()),
            "无法提交界面设置",
        )
    })?;
    Ok(())
}

fn internal_error_with_cause(
    operation: &'static str,
    cause: &'static str,
    message: &'static str,
) -> AppError {
    tracing::warn!(operation, cause, "application service diagnostic");
    AppError::internal(message)
}

fn log_command_result<T>(
    operation: &'static str,
    started_at: Instant,
    result: &Result<T, AppError>,
) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    match result {
        Ok(_) => tracing::info!(operation, status = "success", elapsed_ms),
        Err(error) => tracing::warn!(operation, status = "error", code = error.code, elapsed_ms),
    }
}

fn log_startup_state(operation: &'static str, started_at: Instant, state: &StartupState) {
    let elapsed_ms = started_at.elapsed().as_millis() as u64;
    match state {
        StartupState::OmpReady { .. } => tracing::info!(operation, status = "success", elapsed_ms),
        StartupState::OmpUnavailable { .. } => tracing::warn!(
            operation,
            status = "error",
            code = "omp-unavailable",
            elapsed_ms
        ),
        StartupState::InvalidExecutable {
            diagnostic_code, ..
        }
        | StartupState::VersionFailed {
            diagnostic_code, ..
        }
        | StartupState::ConfigPathFailed {
            diagnostic_code, ..
        } => {
            tracing::warn!(
                operation,
                status = "error",
                code = diagnostic_code,
                elapsed_ms
            )
        }
    }
}

#[tauri::command]
pub fn get_startup_state(service: tauri::State<'_, AppService>) -> StartupState {
    let started_at = Instant::now();
    let state = service.get_startup_state();
    let safe_state = AppService::sanitize_overview_startup_state(&state);
    tracing::info!(
        operation = "get_startup_state",
        status = "success",
        elapsed_ms = started_at.elapsed().as_millis() as u64
    );
    safe_state
}

#[tauri::command]
pub fn get_overview_load(service: tauri::State<'_, AppService>) -> OverviewLoadDto {
    let started_at = Instant::now();
    let result = service.get_overview_load();
    if let Some(error) = result.error.as_ref() {
        tracing::info!(
            operation = "get_overview_load",
            status = "error",
            code = error.code,
            elapsed_ms = started_at.elapsed().as_millis() as u64
        );
    } else {
        tracing::info!(
            operation = "get_overview_load",
            status = "success",
            elapsed_ms = started_at.elapsed().as_millis() as u64
        );
    }
    result
}

#[tauri::command]
pub fn get_ui_settings(service: tauri::State<'_, AppService>) -> Result<AppSettings, AppError> {
    let started_at = Instant::now();
    let result = service.get_ui_settings();
    log_command_result("get_ui_settings", started_at, &result);
    result
}

#[tauri::command]
pub fn save_ui_settings(
    service: tauri::State<'_, AppService>,
    settings: UiSettingsUpdate,
) -> Result<AppSettings, AppError> {
    let started_at = Instant::now();
    let result = service.save_ui_settings(settings);
    log_command_result("save_ui_settings", started_at, &result);
    result
}

#[tauri::command]
pub fn create_custom_provider(
    service: tauri::State<'_, AppService>,
    input: CreateCustomProviderInput,
) -> Result<CreateCustomProviderResult, AppError> {
    let started_at = Instant::now();
    let result = service.create_custom_provider(input);
    log_command_result("create_custom_provider", started_at, &result);
    result
}

#[tauri::command]
pub fn edit_custom_provider(
    service: tauri::State<'_, AppService>,
    input: EditCustomProviderInput,
) -> Result<EditCustomProviderResult, AppError> {
    let started_at = Instant::now();
    let result = service.edit_custom_provider(input);
    log_command_result("edit_custom_provider", started_at, &result);
    result
}

#[tauri::command]
pub fn create_model(
    service: tauri::State<'_, AppService>,
    input: CreateModelInput,
) -> Result<ModelMutationResult, AppError> {
    let started_at = Instant::now();
    let result = service.create_model(input);
    log_command_result("create_model", started_at, &result);
    result
}

#[tauri::command]
pub fn edit_model(
    service: tauri::State<'_, AppService>,
    input: EditModelInput,
) -> Result<ModelMutationResult, AppError> {
    let started_at = Instant::now();
    let result = service.edit_model(input);
    log_command_result("edit_model", started_at, &result);
    result
}

#[tauri::command]
pub fn delete_model(
    service: tauri::State<'_, AppService>,
    input: DeleteModelInput,
) -> Result<ModelMutationResult, AppError> {
    let started_at = Instant::now();
    let result = service.delete_model(input);
    log_command_result("delete_model", started_at, &result);
    result
}
#[tauri::command]
pub fn detect_omp(service: tauri::State<'_, AppService>) -> StartupState {
    let started_at = Instant::now();
    let state = service.detect_omp();
    let safe_state = AppService::sanitize_overview_startup_state(&state);
    log_startup_state("detect_omp", started_at, &safe_state);
    safe_state
}

#[tauri::command]
pub fn validate_selected_omp(
    service: tauri::State<'_, AppService>,
    executable_path: String,
) -> StartupState {
    let started_at = Instant::now();
    let state = service.validate_selected_omp(PathBuf::from(executable_path));
    let safe_state = AppService::sanitize_overview_startup_state(&state);
    log_startup_state("validate_selected_omp", started_at, &safe_state);
    safe_state
}

#[tauri::command]
pub fn initialize_target_configuration(
    service: tauri::State<'_, AppService>,
    executable_path: String,
    expectation: TargetInitializationExpectation,
) -> Result<StartupState, AppError> {
    let started_at = Instant::now();
    let result =
        service.initialize_target_configuration(PathBuf::from(executable_path), expectation);
    let safe_result = result.map(|state| AppService::sanitize_overview_startup_state(&state));
    log_command_result("initialize_target_configuration", started_at, &safe_result);
    safe_result
}

#[tauri::command]
pub fn open_target_configuration_directory(
    service: tauri::State<'_, AppService>,
    app: tauri::AppHandle,
    executable_path: String,
) -> Result<(), AppError> {
    let started_at = Instant::now();
    let result = service
        .target_directory_for_opening(PathBuf::from(executable_path))
        .and_then(|path| {
            app.opener()
                .open_path(path.to_string_lossy(), None::<&str>)
                .map_err(|error| {
                    tracing::warn!(
                        operation = "open_target_configuration_directory",
                        diagnostic = %redact_diagnostic(&error.to_string()),
                        "native directory opener failed"
                    );
                    AppError::new(
                        "target-directory-open-failed",
                        "系统文件管理器未能打开配置目录。",
                        "请检查系统文件管理器关联后重试，并在问题持续时查看脱敏日志。",
                    )
                })
        });
    log_command_result("open_target_configuration_directory", started_at, &result);
    result
}
#[tauri::command]
pub fn confirm_selected_omp(
    service: tauri::State<'_, AppService>,
    executable_path: String,
) -> Result<AppSettings, AppError> {
    let started_at = Instant::now();
    let result = service.confirm_selected_omp(PathBuf::from(executable_path));
    log_command_result("confirm_selected_omp", started_at, &result);
    result
}
