use ambient_recorder::{capture::Native, config::Paths};
use ambient_recorder_cli::{App, Result};
use cli_command_kit::Mode;
use std::{
    io::{self, IsTerminal, Write},
    path::PathBuf,
};
use tokio_util::sync::CancellationToken;
fn run() -> Result<()> {
    let mode = Mode::from_env();
    let Some(prepared) =
        ambient_recorder_cli::prepare(std::env::args_os(), mode, &mut io::stdout())?
    else {
        return Ok(());
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let token = CancellationToken::new();
    let signal_token = token.clone();
    let home = dirs::home_dir()
        .ok_or_else(|| ambient_recorder::Error::Invalid("home directory is unavailable".into()))?;
    let paths = if prepared.uses_configuration() {
        Paths::from_environment()?
    } else {
        Paths::new(home.clone(), None, None)?
    };
    let mut app = App::new(paths, home, std::env::current_exe()?, mode);
    app.service_socket = std::env::var_os(ambient_recorder::service_info::ENVIRONMENT)
        .filter(|p| !p.is_empty())
        .map(PathBuf::from);
    app.color = mode == Mode::Human
        && io::stderr().is_terminal()
        && std::env::var_os("NO_COLOR").is_none_or(|v| v.is_empty())
        && std::env::var("TERM").unwrap_or_default() != "dumb";
    runtime.block_on(async {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        let listener = tokio::spawn(async move {
            tokio::select! {_=tokio::signal::ctrl_c()=>{},_=term.recv()=>{}}
            signal_token.cancel();
        });
        let result = app
            .execute_prepared(
                prepared,
                &mut io::stdout(),
                Some(Box::new(io::stderr())),
                &token,
                &mut Native,
            )
            .await;
        listener.abort();
        let _ = listener.await;
        result
    })
}
fn main() {
    if let Err(error) = run() {
        let prefix = if Mode::from_env() == Mode::Agent {
            "ERR:"
        } else {
            "[ERROR]"
        };
        let _ = writeln!(
            io::stderr(),
            "{prefix} {}",
            cli_command_kit::display_message(&error.to_string())
        );
        std::process::exit(1)
    }
}
