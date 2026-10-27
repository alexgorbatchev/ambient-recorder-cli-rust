//! Per-user launchd supervision. Filesystem and process destinations are explicit.
mod plist_file;
mod process;
mod status;
use crate::{Error, Result, service_info};
pub use status::{RecordingPaths, ServiceStatus};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub const LABEL: &str = "com.alexgorbatchev.ambient-recorder";
#[derive(Clone, Debug)]
pub struct AgentSettings {
    pub output: PathBuf,
    pub config: Option<PathBuf>,
}
#[derive(Clone, Copy, Debug)]
pub enum Action {
    Start,
    Stop,
    Restart,
    Uninstall,
}
#[derive(Clone, Debug)]
pub struct Agent {
    pub plist: PathBuf,
    pub domain: String,
    pub label: String,
    pub binary: PathBuf,
    pub control: PathBuf,
}
impl Agent {
    pub fn new(
        plist: PathBuf,
        domain: String,
        label: String,
        binary: PathBuf,
        control: PathBuf,
    ) -> Result<Self> {
        if !plist.is_absolute()
            || !binary.is_absolute()
            || !control.is_absolute()
            || domain.is_empty()
            || label.is_empty()
        {
            return Err(Error::Invalid(
                "service paths must be absolute and its domain and label nonempty".into(),
            ));
        }
        Ok(Self {
            plist,
            domain,
            label,
            binary,
            control,
        })
    }
    pub fn user(home: &Path, binary: &Path) -> Result<Self> {
        // SAFETY: getuid has no preconditions and observes the real login identity.
        let uid = unsafe { libc::getuid() };
        if uid == 0 {
            return Err(Error::Invalid(
                "run service commands as your logged-in user, without sudo".into(),
            ));
        }
        Self::new(
            home.join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist")),
            format!("gui/{uid}"),
            LABEL.into(),
            fs::canonicalize(binary)?,
            "/bin/launchctl".into(),
        )
    }
    fn target(&self) -> String {
        format!("{}/{}", self.domain, self.label)
    }
    pub fn installed(&self) -> Result<bool> {
        match fs::symlink_metadata(&self.plist) {
            Ok(meta) if meta.is_file() => Ok(true),
            Ok(_) => Err(Error::Invalid(format!(
                "service file must be a regular file: {}",
                self.plist.display()
            ))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(e.into()),
        }
    }
    pub async fn loaded(&self, token: &CancellationToken) -> Result<Option<Vec<u8>>> {
        match self
            .command(&["print".into(), self.target().into()], token)
            .await
        {
            Ok(out) => Ok(Some(out)),
            Err(Error::Command { status, .. }) if status.code() == Some(113) => Ok(None),
            Err(e) => Err(e),
        }
    }
    pub async fn install(&self, settings: &AgentSettings, token: &CancellationToken) -> Result<()> {
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        let value = self.configuration(settings)?;
        plist_file::create_directory(
            self.plist
                .parent()
                .ok_or_else(|| Error::Invalid("service path has no parent".into()))?,
        )?;
        plist_file::create_directory(&settings.output)?;
        plist_file::create(&self.plist, &value)?;
        self.start(false, token).await.map_err(|e| {
            Error::Multiple(vec![
                Error::Invalid(format!(
                    "service file saved at {}; correct the error and retry service start",
                    self.plist.display()
                )),
                e,
            ])
        })
    }
    pub async fn start(&self, restart: bool, token: &CancellationToken) -> Result<()> {
        if !self.installed()? {
            return Err(Error::Invalid(
                "service is not installed; run service install first".into(),
            ));
        }
        if self.refresh(token).await? || restart {
            self.unload(token).await?;
        }
        self.command(&["enable".into(), self.target().into()], token)
            .await?;
        let args = if self.loaded(token).await?.is_none() {
            vec![
                "bootstrap".into(),
                self.domain.clone().into(),
                self.plist.clone().into_os_string(),
            ]
        } else {
            vec!["kickstart".into(), self.target().into()]
        };
        self.command(&args, token).await?;
        Ok(())
    }
    pub async fn unload(&self, token: &CancellationToken) -> Result<()> {
        let operation = async {
            if self.loaded(token).await?.is_none() {
                return Ok(());
            }
            self.command(&["bootout".into(), self.target().into()], token)
                .await?;
            while self.loaded(token).await?.is_some() {
                tokio::select! {biased;_=token.cancelled()=>return Err(Error::Cancelled),_=tokio::time::sleep(Duration::from_millis(50))=>{}}
            }
            Ok(())
        };
        tokio::time::timeout(process::COMMAND_TIMEOUT, operation)
            .await
            .map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "wait for service shutdown timed out",
                )
            })?
    }
    pub async fn manage(&self, action: Action, token: &CancellationToken) -> Result<()> {
        match action {
            Action::Start => self.start(false, token).await,
            Action::Restart => self.start(true, token).await,
            Action::Stop | Action::Uninstall => {
                self.command(&["disable".into(), self.target().into()], token)
                    .await?;
                self.unload(token).await?;
                if matches!(action, Action::Uninstall) {
                    match fs::remove_file(&self.plist) {
                        Ok(()) => {}
                        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                        Err(e) => return Err(e.into()),
                    }
                }
                Ok(())
            }
        }
    }
    pub fn configuration(&self, settings: &AgentSettings) -> Result<plist::Value> {
        if !settings.output.is_absolute()
            || settings.config.as_ref().is_some_and(|p| !p.is_absolute())
        {
            return Err(Error::Invalid(
                "saved service paths must be absolute".into(),
            ));
        }
        let mut args = vec![
            plist_file::path_value(&self.binary)?,
            "recording".into(),
            "start".into(),
            "--output".into(),
            plist_file::path_value(&settings.output)?,
        ];
        if let Some(config) = &settings.config {
            args.extend(["--config".into(), plist_file::path_value(config)?]);
        }
        let mut fields = plist::Dictionary::new();
        fields.insert("Label".into(), self.label.clone().into());
        fields.insert("ProgramArguments".into(), plist::Value::Array(args));
        fields.insert("KeepAlive".into(), true.into());
        fields.insert("LimitLoadToSessionType".into(), "Aqua".into());
        fields.insert("ThrottleInterval".into(), 5i64.into());
        fields.insert("Umask".into(), 0o077i64.into());
        fields.insert(
            "StandardErrorPath".into(),
            plist_file::path_value(&settings.output.join("supervisor.stderr.log"))?,
        );
        let mut env = plist::Dictionary::new();
        env.insert(
            service_info::ENVIRONMENT.into(),
            plist_file::path_value(&service_info::socket_path(&self.plist))?,
        );
        fields.insert("EnvironmentVariables".into(), plist::Value::Dictionary(env));
        Ok(plist::Value::Dictionary(fields))
    }
}
