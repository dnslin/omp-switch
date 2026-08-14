use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::Arc,
    time::Instant,
};

use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::{
    error::AppError,
    omp_environment::{OmpEnvironment, SystemOmpEnvironment, TargetAccess},
    redaction::redact_diagnostic,
};

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
        target_configuration: String,
        previous_target_configuration: Option<String>,
        target_access: TargetAccess,
        requires_confirmation: bool,
    },
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

#[derive(Clone)]
pub struct AppService {
    settings_path: Arc<PathBuf>,
    settings: Arc<RwLock<AppSettings>>,
    settings_write: Arc<parking_lot::Mutex<()>>,
    environment: Arc<dyn OmpEnvironment>,
    pending_omp: Arc<RwLock<Option<PathBuf>>>,
}

impl AppService {
    pub fn new(settings_path: PathBuf) -> Result<Self, AppError> {
        Self::new_with_environment(settings_path, Arc::new(SystemOmpEnvironment))
    }

    pub fn new_with_environment(
        settings_path: PathBuf,
        environment: Arc<dyn OmpEnvironment>,
    ) -> Result<Self, AppError> {
        let settings = load_settings(&settings_path)?;
        Ok(Self {
            settings_path: Arc::new(settings_path),
            settings: Arc::new(RwLock::new(settings)),
            settings_write: Arc::new(parking_lot::Mutex::new(())),
            environment,
            pending_omp: Arc::new(RwLock::new(None)),
        })
    }

    pub fn get_startup_state(&self) -> StartupState {
        self.detect_omp()
    }

    pub fn detect_omp(&self) -> StartupState {
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
            } => Some(target_configuration),
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
        let target_access = match self.environment.inspect_target(&target) {
            Ok(access) => access,
            Err(error) => {
                return StartupState::ConfigPathFailed {
                    executable_path,
                    version,
                    message: "权威配置目录及其父目录不可访问。OMP Switch 不会改用其他目录。"
                        .to_owned(),
                    diagnostic_code: io_error_cause(error.kind()).to_owned(),
                    exit_code: None,
                    stderr: String::new(),
                };
            }
        };
        StartupState::OmpReady {
            executable_path,
            version,
            target_configuration: target.to_string_lossy().into_owned(),
            previous_target_configuration,
            target_access,
            requires_confirmation,
        }
    }

    pub fn confirm_selected_omp(&self, executable: PathBuf) -> Result<AppSettings, AppError> {
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

fn io_error_cause(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "io-not-found",
        std::io::ErrorKind::PermissionDenied => "io-permission-denied",
        std::io::ErrorKind::AlreadyExists => "io-already-exists",
        std::io::ErrorKind::InvalidInput => "io-invalid-input",
        std::io::ErrorKind::InvalidData => "io-invalid-data",
        std::io::ErrorKind::WriteZero => "io-write-zero",
        std::io::ErrorKind::StorageFull => "io-storage-full",
        _ => "io-other",
    }
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
    tracing::info!(
        operation = "get_startup_state",
        status = "success",
        elapsed_ms = started_at.elapsed().as_millis() as u64
    );
    state
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
pub fn detect_omp(service: tauri::State<'_, AppService>) -> StartupState {
    let started_at = Instant::now();
    let state = service.detect_omp();
    log_startup_state("detect_omp", started_at, &state);
    state
}

#[tauri::command]
pub fn validate_selected_omp(
    service: tauri::State<'_, AppService>,
    executable_path: String,
) -> StartupState {
    let started_at = Instant::now();
    let state = service.validate_selected_omp(PathBuf::from(executable_path));
    log_startup_state("validate_selected_omp", started_at, &state);
    state
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
