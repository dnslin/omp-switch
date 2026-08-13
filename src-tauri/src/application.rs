use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use serde::{Deserialize, Serialize};

use crate::error::AppError;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum StartupState {
    OmpUnavailable { message: String },
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

#[derive(Clone)]
pub struct AppService {
    settings_path: Arc<PathBuf>,
    settings: Arc<RwLock<AppSettings>>,
}

impl AppService {
    pub fn new(settings_path: PathBuf) -> Result<Self, AppError> {
        let settings = load_settings(&settings_path)?;
        Ok(Self {
            settings_path: Arc::new(settings_path),
            settings: Arc::new(RwLock::new(settings)),
        })
    }

    pub fn get_startup_state(&self) -> StartupState {
        StartupState::OmpUnavailable {
            message: "尚未检测 OMP".to_owned(),
        }
    }

    pub fn get_ui_settings(&self) -> Result<AppSettings, AppError> {
        self.settings
            .read()
            .map(|settings| settings.clone())
            .map_err(|_| AppError::internal("无法读取界面设置"))
    }

    pub fn save_ui_settings(&self, settings: AppSettings) -> Result<AppSettings, AppError> {
        persist_settings(&self.settings_path, &settings)?;
        let mut current = self
            .settings
            .write()
            .map_err(|_| AppError::internal("无法保存界面设置"))?;
        *current = settings.clone();
        Ok(settings)
    }
}

fn load_settings(path: &Path) -> Result<AppSettings, AppError> {
    match fs::read(path) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|_| AppError::internal("界面设置文件无法解析"))
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(AppSettings::default()),
        Err(_) => Err(AppError::internal("无法读取界面设置文件")),
    }
}

fn persist_settings(path: &Path, settings: &AppSettings) -> Result<(), AppError> {
    let parent = path
        .parent()
        .ok_or_else(|| AppError::internal("界面设置路径无效"))?;
    fs::create_dir_all(parent).map_err(|_| AppError::internal("无法创建应用数据目录"))?;
    let bytes = serde_json::to_vec_pretty(settings)
        .map_err(|_| AppError::internal("无法序列化界面设置"))?;
    let temporary_path = path.with_extension("json.tmp");
    fs::write(&temporary_path, bytes).map_err(|_| AppError::internal("无法写入界面设置"))?;
    fs::rename(&temporary_path, path).map_err(|_| AppError::internal("无法提交界面设置"))?;
    Ok(())
}

#[tauri::command]
pub fn get_startup_state(service: tauri::State<'_, AppService>) -> StartupState {
    tracing::info!(
        operation = "get_startup_state",
        "application service completed"
    );
    service.get_startup_state()
}

#[tauri::command]
pub fn get_ui_settings(service: tauri::State<'_, AppService>) -> Result<AppSettings, AppError> {
    let result = service.get_ui_settings();
    match &result {
        Ok(_) => tracing::info!(operation = "get_ui_settings", status = "success"),
        Err(error) => tracing::warn!(
            operation = "get_ui_settings",
            status = "error",
            code = error.code
        ),
    }
    result
}

#[tauri::command]
pub fn save_ui_settings(
    service: tauri::State<'_, AppService>,
    settings: AppSettings,
) -> Result<AppSettings, AppError> {
    let result = service.save_ui_settings(settings);
    match &result {
        Ok(_) => tracing::info!(operation = "save_ui_settings", status = "success"),
        Err(error) => tracing::warn!(
            operation = "save_ui_settings",
            status = "error",
            code = error.code
        ),
    }
    result
}
