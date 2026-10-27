//! Native audio recording primitives and lifecycle management.
pub mod audio;
pub mod capture;
pub mod config;
pub mod journal;
pub mod launchd;
pub mod microphone;
pub mod recording;
pub mod service_info;
pub mod storage;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Toml(#[from] toml::de::Error),
    #[error(transparent)]
    EncodeToml(#[from] toml::ser::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Plist(#[from] plist::Error),
    #[error("launchctl {arguments}: {status}: {}", String::from_utf8_lossy(.output).trim())]
    Command {
        arguments: String,
        status: std::process::ExitStatus,
        output: Vec<u8>,
    },
    #[error("no recording file is currently open")]
    NoCurrentFile,
    #[error("audio codec: {0}")]
    Codec(String),
    #[error("operation cancelled")]
    Cancelled,
    #[error("native capture requires macOS 14.2 or later")]
    UnsupportedPlatform,
    #[error("capture cleanup failed; process restart required: {0}")]
    Cleanup(String),
    #[error("{}", display_errors(.0))]
    Multiple(Vec<Error>),
}
pub(crate) fn display_errors(errors: &[Error]) -> String {
    errors
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
pub type Result<T> = std::result::Result<T, Error>;
