use super::Agent;
use crate::{Error, Result, service_info};
use std::path::PathBuf;
use tokio_util::sync::CancellationToken;
#[derive(Clone, Debug)]
pub struct RecordingPaths {
    pub output: String,
    pub config: String,
    pub current: String,
    pub source: String,
}
#[derive(Clone, Debug)]
pub struct ServiceStatus {
    pub installed: bool,
    pub loaded: bool,
    pub service_version: String,
    pub microphone: String,
    pub paths: RecordingPaths,
    pub diagnostics: Vec<u8>,
}
impl Agent {
    /// Saved arguments are resolved by the consuming CLI's own command parser.
    pub async fn status(
        &self,
        token: &CancellationToken,
        resolve: &impl Fn(&[String]) -> Result<(PathBuf, PathBuf)>,
    ) -> Result<ServiceStatus> {
        let installed = self.installed()?;
        let native = self.loaded(token).await?;
        let loaded = native.is_some();
        let mut status = ServiceStatus {
            installed,
            loaded,
            service_version: if loaded { "unavailable" } else { "not running" }.into(),
            microphone: if loaded { "unavailable" } else { "not running" }.into(),
            paths: RecordingPaths {
                output: "unavailable".into(),
                config: "unavailable".into(),
                current: if loaded { "unavailable" } else { "not running" }.into(),
                source: "unavailable".into(),
            },
            diagnostics: native.unwrap_or_default(),
        };
        if loaded {
            match service_info::query(&service_info::socket_path(&self.plist), token).await {
                Ok(live) => {
                    status.service_version = live.version;
                    if let Some(mic) = live.microphone {
                        status.microphone = if mic.is_empty() { "none".into() } else { mic };
                    }
                    if !live.output_dir.is_empty() {
                        status.paths = RecordingPaths {
                            output: live.output_dir,
                            config: if live.config_path.is_empty() {
                                "none (defaults)".into()
                            } else {
                                live.config_path
                            },
                            current: if live.current_file.is_empty() {
                                "none".into()
                            } else {
                                live.current_file
                            },
                            source: "live".into(),
                        };
                        return Ok(status);
                    }
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
        if installed {
            match self.saved_paths(token, resolve).await {
                Ok((output, config)) => {
                    status.paths.output = output.display().to_string();
                    status.paths.config = config.display().to_string();
                    status.paths.source = "saved".into();
                }
                Err(Error::Cancelled) => return Err(Error::Cancelled),
                Err(_) => {}
            }
        }
        Ok(status)
    }
    async fn saved_paths(
        &self,
        token: &CancellationToken,
        resolve: &impl Fn(&[String]) -> Result<(PathBuf, PathBuf)>,
    ) -> Result<(PathBuf, PathBuf)> {
        let value = self.read_configuration(token).await?;
        let args = value
            .as_dictionary()
            .and_then(|d| d.get("ProgramArguments"))
            .and_then(plist::Value::as_array)
            .ok_or_else(|| Error::Invalid("service has no recording arguments".into()))?;
        let args = args
            .iter()
            .map(|v| {
                v.as_string().map(str::to_owned).ok_or_else(|| {
                    Error::Invalid("service executable arguments must be strings".into())
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let paths = resolve(&args)?;
        if !paths.0.is_absolute() || !paths.1.is_absolute() {
            return Err(Error::Invalid(
                "saved service paths must be absolute".into(),
            ));
        }
        Ok(paths)
    }
}
