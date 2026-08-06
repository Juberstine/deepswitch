use std::path::PathBuf;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, AppError>;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("could not determine the user's home directory")]
    HomeDirectoryUnavailable,

    #[error("CODEX_HOME must be a non-empty absolute path")]
    InvalidCodexHome,

    #[error("failed to read or write {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("invalid TOML in {path}: {source}")]
    Toml {
        path: PathBuf,
        #[source]
        source: toml_edit::TomlError,
    },

    #[error("invalid JSON in {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },

    #[error("credential-store error: {0}")]
    CredentialStore(String),

    #[error(
        "the DeepSeek API key is not in the OS keychain; run `codex-deepseek-switcher key set`"
    )]
    CredentialMissing,

    #[error("the API key must be non-empty and start with `sk-`")]
    InvalidApiKey,

    #[error(
        "no saved Codex selection exists; run `codex-deepseek-switcher setup` or `use deepseek` first"
    )]
    OriginalStateMissing,

    #[error(
        "DeepSeek is already selected but no switcher state exists; restore your Codex provider manually before setup so the original selection can be captured safely"
    )]
    DeepSeekSelectedWithoutState,

    #[error("unsupported value for top-level Codex setting `{0}`; expected a string")]
    UnsupportedSetting(String),

    #[error("unsupported Codex configuration structure: {0}")]
    UnsupportedConfigStructure(String),

    #[error("refusing to replace symbolic link {0}")]
    SymbolicLink(PathBuf),

    #[error("the embedded DeepSeek model catalog is invalid: {0}")]
    InvalidEmbeddedCatalog(String),
}

pub(crate) fn io_error(path: impl Into<PathBuf>, source: std::io::Error) -> AppError {
    AppError::Io {
        path: path.into(),
        source,
    }
}
