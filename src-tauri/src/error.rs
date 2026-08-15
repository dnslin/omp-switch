use serde::Serialize;
use thiserror::Error;

#[derive(Clone, Debug, Error, PartialEq, Eq, Serialize)]
#[error("{message}")]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    pub code: &'static str,
    pub message: String,
    pub action: String,
}

impl AppError {
    pub fn new(code: &'static str, message: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            action: action.into(),
        }
    }
    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            code: "internal-error",
            message: message.into(),
            action: "请重试；如果问题持续，请查看脱敏日志。".to_owned(),
        }
    }
}

pub(crate) fn io_error_cause(kind: std::io::ErrorKind) -> &'static str {
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
