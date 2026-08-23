use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("ollama: {0}")]
    Ollama(String),

    #[error("sidecar: {0}")]
    Sidecar(String),

    #[error("config: {0}")]
    Config(String),

    #[error("not yet implemented: {0}")]
    NotImplemented(&'static str),

    #[error("{0}")]
    Other(String),
}

impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        let code = match self {
            AppError::Io(_) => "io",
            AppError::Http(_) => "http",
            AppError::Sqlite(_) => "sqlite",
            AppError::Json(_) => "json",
            AppError::Ollama(_) => "ollama",
            AppError::Sidecar(_) => "sidecar",
            AppError::Config(_) => "config",
            AppError::NotImplemented(_) => "not_implemented",
            AppError::Other(_) => "other",
        };
        // Sqlite/Http error Display text comes from the driver, not from a
        // code path we control the wording of, so it gets the same
        // key-pattern redaction as cloud provider error bodies before it
        // reaches the frontend.
        let message = match self {
            AppError::Sqlite(_) | AppError::Http(_) => {
                crate::inference::cloud::sanitize_error_body(&self.to_string())
            }
            _ => self.to_string(),
        };
        use serde::ser::SerializeStruct;
        let mut st = s.serialize_struct("AppError", 2)?;
        st.serialize_field("code", code)?;
        st.serialize_field("message", &message)?;
        st.end()
    }
}

pub type AppResult<T> = std::result::Result<T, AppError>;
