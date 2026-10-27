use crate::{Error, Result, cli, completion, output};
use ambient_recorder::{
    capture::Backend,
    config::{Overrides, Paths, Settings},
    launchd::{Action, Agent, AgentSettings},
    recording::{self, RunConfig},
    service_info::{Info, Server},
    storage,
};
use cli_command_kit::{
    Mode,
    clap::{ArgMatches, error::ErrorKind, parser::ValueSource},
};
use std::{
    ffi::OsString,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};
use tokio_util::sync::CancellationToken;

pub struct Prepared {
    matches: ArgMatches,
}
impl Prepared {
    pub fn uses_configuration(&self) -> bool {
        match self.matches.subcommand() {
            Some(("config" | "recording", _)) => true,
            Some(("service", group)) => {
                matches!(group.subcommand_name(), Some("print" | "install"))
            }
            _ => false,
        }
    }
}
pub fn prepare<I, T>(args: I, mode: Mode, out: &mut dyn Write) -> Result<Option<Prepared>>
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let command = cli::command(mode);
    let matches = match command.clone().try_get_matches_from(args) {
        Ok(m) => m,
        Err(e) if matches!(e.kind(), ErrorKind::DisplayHelp | ErrorKind::DisplayVersion) => {
            out.write_all(e.to_string().as_bytes())?;
            return Ok(None);
        }
        Err(e) => return Err(Error::Invalid(e.to_string())),
    };
    if matches.get_flag("version") {
        writeln!(out, "{}", env!("CARGO_PKG_VERSION"))?;
        return Ok(None);
    }
    if matches.subcommand_name() == Some("skill") {
        out.write_all(cli::SKILL.as_bytes())?;
        return Ok(None);
    }
    let mut current = &matches;
    let mut selected = &command;
    while let Some((name, child)) = current.subcommand() {
        selected = selected.find_subcommand(name).expect("parsed command");
        current = child;
    }
    if selected.has_subcommands() {
        writeln!(out, "{}", selected.clone().render_help())?;
        return Ok(None);
    }
    if current.try_contains_id("output").unwrap_or(false)
        && current.value_source("output") == Some(ValueSource::CommandLine)
        && current
            .get_one::<PathBuf>("output")
            .is_some_and(|p| p.as_os_str().is_empty())
    {
        return Err(Error::Invalid("--output must be nonempty".into()));
    }
    Ok(Some(Prepared { matches }))
}

pub struct App {
    pub paths: Paths,
    pub home: PathBuf,
    pub binary: PathBuf,
    pub mode: Mode,
    pub color: bool,
    pub service: Option<Agent>,
    pub service_socket: Option<PathBuf>,
}
impl App {
    pub fn new(paths: Paths, home: PathBuf, binary: PathBuf, mode: Mode) -> Self {
        Self {
            paths,
            home,
            binary,
            mode,
            color: false,
            service: None,
            service_socket: None,
        }
    }
    pub async fn execute<I, T>(
        &mut self,
        args: I,
        out: &mut dyn Write,
        console: Option<Box<dyn Write>>,
        token: &CancellationToken,
        backend: &mut dyn Backend,
    ) -> Result<()>
    where
        I: IntoIterator<Item = T>,
        T: Into<OsString> + Clone,
    {
        let Some(prepared) = prepare(args, self.mode, out)? else {
            return Ok(());
        };
        self.execute_prepared(prepared, out, console, token, backend)
            .await
    }
    pub async fn execute_prepared(
        &mut self,
        prepared: Prepared,
        out: &mut dyn Write,
        console: Option<Box<dyn Write>>,
        token: &CancellationToken,
        backend: &mut dyn Backend,
    ) -> Result<()> {
        let Prepared { matches } = prepared;
        let config = matches
            .get_one::<PathBuf>("config")
            .expect("default config path");
        let Some((subject, group)) = matches.subcommand() else {
            unreachable!("root has subcommands")
        };
        let Some((verb, leaf)) = group.subcommand() else {
            unreachable!("group help handled")
        };
        match (subject, verb) {
            ("config", "init") => {
                let path = Settings::create(&self.paths, Some(config))?;
                let path = cli_command_kit::fold_home(&path, &self.home);
                if self.mode == Mode::Agent {
                    writeln!(out, "config={path}")?
                } else {
                    writeln!(out, "Configuration created: {path}")?
                }
            }
            ("config", "print-file") => {
                let path = self.paths.resolve_config(Some(config))?;
                if !std::fs::metadata(&path)?.is_file() {
                    return Err(Error::Invalid(format!(
                        "configuration must be a regular file: {}",
                        path.display()
                    )));
                }
                self.print_path(out, &path)?;
            }
            ("microphone", "list") => {
                output::microphones(out, &backend.microphones()?, self.mode, &self.home)?
            }
            ("recording", "print-dir" | "print-file") => {
                let (settings, _) = self.settings(config, leaf)?;
                let directory = std::path::absolute(&settings.output.dir)?;
                let path = if verb == "print-file" {
                    storage::current_file(&directory)?
                } else {
                    directory
                };
                self.print_path(out, &path)?;
            }
            ("recording", "start") => {
                let (settings, loaded) = self.settings(config, leaf)?;
                self.record(settings, loaded, console, token, backend)
                    .await?;
            }
            ("service", verb) => self.service_command(verb, leaf, config, out, token).await?,
            ("completion", shell) => {
                completion::write(shell, leaf.get_flag("no-descriptions"), out)?
            }
            _ => unreachable!("complete command dispatch"),
        }
        Ok(())
    }
    fn print_path(&self, out: &mut dyn Write, path: &Path) -> Result<()> {
        writeln!(out, "{}", cli_command_kit::fold_home(path, &self.home))?;
        Ok(())
    }
    fn settings(&self, config: &Path, m: &ArgMatches) -> Result<(Settings, Option<PathBuf>)> {
        let supplied = |key: &str| {
            m.try_contains_id(key).unwrap_or(false)
                && m.value_source(key) == Some(ValueSource::CommandLine)
        };
        // Validate explicit overrides before any configuration-file access.
        let output = if supplied("output") {
            let path = m.get_one::<PathBuf>("output").expect("output path").clone();
            if path.as_os_str().is_empty() {
                return Err(Error::Invalid("--output must be nonempty".into()));
            }
            Some(path)
        } else {
            None
        };
        let interval = if supplied("sync-interval") {
            Some(format!(
                "{}ns",
                m.get_one::<Duration>("sync-interval")
                    .expect("parsed interval")
                    .as_nanos()
            ))
        } else {
            None
        };
        let (mut settings, loaded) = Settings::load(&self.paths, Some(config))?;
        settings.apply(Overrides {
            output,
            bitrate: if supplied("bitrate") {
                m.get_one::<i32>("bitrate").copied()
            } else {
                None
            },
            complexity: if supplied("complexity") {
                m.get_one::<i32>("complexity").copied()
            } else {
                None
            },
            sync_interval: interval,
        });
        settings.validate()?;
        Ok((settings, loaded))
    }
    async fn record(
        &self,
        mut settings: Settings,
        loaded: Option<PathBuf>,
        console: Option<Box<dyn Write>>,
        token: &CancellationToken,
        backend: &mut dyn Backend,
    ) -> Result<()> {
        settings.output.dir = std::path::absolute(&settings.output.dir)?;
        let mut config = RunConfig::new(settings);
        config.agent = self.mode == Mode::Agent;
        config.color = self.color;
        let mut server = if let Some(path) = &self.service_socket {
            Some(
                Server::listen(
                    path,
                    Info {
                        version: env!("CARGO_PKG_VERSION").into(),
                        output_dir: config.settings.output.dir.display().to_string(),
                        config_path: loaded.map(|p| p.display().to_string()).unwrap_or_default(),
                        current_file: String::new(),
                        microphone: Some(String::new()),
                    },
                )
                .await?,
            )
        } else {
            None
        };
        if let Some(server) = &server {
            let state = server.state();
            let current = state.clone();
            config.on_current = Some(Box::new(move |path| {
                current.set_current_file(path.map(|p| p.display().to_string()).unwrap_or_default())
            }));
            config.on_microphone = Some(Box::new(move |name| state.set_microphone(name)));
        }
        // This future is driven on the calling thread; native capture is never moved
        // onto a generic blocking pool. Tokio workers own the private HTTP server.
        let result = recording::run(backend, config, console, token);
        let cleanup = if let Some(server) = server.as_mut() {
            server.close().await
        } else {
            Ok(())
        };
        match (result, cleanup) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(e), Ok(())) | (Ok(()), Err(e)) => Err(e),
            (Err(a), Err(b)) => Err(Error::Multiple(vec![a, b])),
        }
    }
    fn agent(&self, preview: bool) -> Result<Agent> {
        if let Some(agent) = &self.service {
            return Ok(agent.clone());
        }
        if preview {
            // getuid has no preconditions and identifies the actual user domain.
            let uid = unsafe { libc::getuid() };
            Agent::new(
                self.home
                    .join("Library/LaunchAgents")
                    .join(format!("{}.plist", ambient_recorder::launchd::LABEL)),
                format!("gui/{uid}"),
                ambient_recorder::launchd::LABEL.into(),
                std::fs::canonicalize(&self.binary)?,
                "/bin/launchctl".into(),
            )
        } else {
            Agent::user(&self.home, &self.binary)
        }
    }
    async fn service_command(
        &self,
        verb: &str,
        m: &ArgMatches,
        config: &Path,
        out: &mut dyn Write,
        token: &CancellationToken,
    ) -> Result<()> {
        let agent = self.agent(verb == "print")?;
        match verb {
            "print" | "install" => {
                let (settings, mut loaded) = self.settings(config, m)?;
                if verb == "install" && loaded.is_none() {
                    loaded = Some(Settings::create(&self.paths, Some(config))?)
                }
                let settings = AgentSettings {
                    output: std::path::absolute(settings.output.dir)?,
                    config: loaded,
                };
                if verb == "print" {
                    agent.configuration(&settings)?.to_writer_xml(out)?;
                } else {
                    agent.install(&settings, token).await?;
                    output::service_result(out, verb, &agent, self.mode, &self.home)?;
                }
            }
            "status" => {
                let status = agent.status(token, &saved_paths).await?;
                output::status(
                    out,
                    &status,
                    &agent,
                    self.mode,
                    &self.home,
                    m.get_flag("details"),
                )?;
            }
            action => {
                let action = match action {
                    "start" => Action::Start,
                    "stop" => Action::Stop,
                    "restart" => Action::Restart,
                    "uninstall" => Action::Uninstall,
                    _ => unreachable!("parsed service action"),
                };
                agent.manage(action, token).await?;
                output::service_result(out, verb, &agent, self.mode, &self.home)?;
            }
        }
        Ok(())
    }
}
pub fn saved_paths(args: &[String]) -> Result<(PathBuf, PathBuf)> {
    let m = cli::command(Mode::Agent)
        .try_get_matches_from(args)
        .map_err(|e| Error::Invalid(e.to_string()))?;
    let leaf = m
        .subcommand_matches("recording")
        .and_then(|m| m.subcommand_matches("start"))
        .ok_or_else(|| Error::Invalid("service must invoke recording start".into()))?;
    let output = leaf
        .get_one::<PathBuf>("output")
        .expect("default output")
        .clone();
    let config = m
        .get_one::<PathBuf>("config")
        .expect("default config")
        .clone();
    if !output.is_absolute() || !config.is_absolute() {
        return Err(Error::Invalid(
            "saved service paths must be absolute".into(),
        ));
    }
    Ok((output, config))
}
