use cli_command_kit::{
    HelpOptions, Mode, TreeOptions,
    clap::{
        Arg, ArgAction, Command,
        builder::{OsStringValueParser, TypedValueParser},
    },
};
use std::path::PathBuf;
pub const SKILL: &str = include_str!("../skills/ambient-recorder/SKILL.md");
pub fn help_options() -> HelpOptions {
    HelpOptions {
        skill: Some(SKILL.into()),
        tree: TreeOptions {
            terminal_width: Some(match cli_command_kit::terminal_width() {
                0 => 80,
                w => w,
            }),
            ..Default::default()
        },
        ..Default::default()
    }
}
fn cmd(name: &'static str, about: &'static str) -> Command {
    Command::new(name).about(about)
}
fn flag(name: &'static str, about: &'static str) -> Arg {
    Arg::new(name)
        .long(name)
        .action(ArgAction::SetTrue)
        .num_args(0..=1)
        .require_equals(true)
        .default_missing_value("true")
        .help(about)
}
fn output() -> Arg {
    Arg::new("output")
        .long("output")
        .default_value("")
        .value_parser(OsStringValueParser::new().map(PathBuf::from))
        .help("Recording directory; empty default inherits configured output")
}
pub fn command(mode: Mode) -> Command {
    let root = cmd(
        "ambient-recorder",
        "Record microphone and computer playback locally",
    )
    .disable_version_flag(true)
    .version(env!("CARGO_PKG_VERSION"))
    .arg(flag("version", "Print only the raw version").short('v'))
    .arg(
        Arg::new("config")
            .long("config")
            .global(true)
            .default_value("")
            .value_parser(OsStringValueParser::new().map(PathBuf::from))
            .help("TOML file; empty selects XDG configuration"),
    )
    .subcommand(
        cmd("config", "Manage recorder configuration")
            .subcommand(cmd("init", "Create default TOML without replacing a file"))
            .subcommand(cmd("print-file", "Print the existing configuration path")),
    )
    .subcommand(
        cmd("microphone", "Inspect microphone inputs").subcommand(cmd(
            "list",
            "List available microphones and preference identifiers",
        )),
    )
    .subcommand(
        cmd("recording", "Capture microphone and computer playback")
            .subcommand(
                cmd("start", "Record continuously until interrupted")
                    .arg(output())
                    .arg(
                        Arg::new("bitrate")
                            .long("bitrate")
                            .default_value("32000")
                            .value_parser(
                                cli_command_kit::clap::value_parser!(i32).range(6000..=128000),
                            )
                            .help("Opus bitrate in bits per second"),
                    )
                    .arg(
                        Arg::new("complexity")
                            .long("complexity")
                            .default_value("2")
                            .value_parser(cli_command_kit::clap::value_parser!(i32).range(0..=10))
                            .help("Encoder effort; higher uses more CPU"),
                    )
                    .arg(
                        Arg::new("sync-interval")
                            .long("sync-interval")
                            .default_value("5s")
                            .value_parser(ambient_recorder::config::parse_interval)
                            .help("Positive interval between storage synchronization attempts"),
                    ),
            )
            .subcommand(cmd("print-dir", "Print the configured recording directory"))
            .subcommand(cmd("print-file", "Print the verified open recording file").arg(output())),
    )
    .subcommand(
        cmd(
            "service",
            "Manage background recording and startup at login",
        )
        .subcommand(
            cmd(
                "print",
                "Print a macOS service configuration without installing",
            )
            .arg(output()),
        )
        .subcommand(
            cmd(
                "install",
                "Install startup at login and request recording now",
            )
            .arg(output()),
        )
        .subcommand(cmd(
            "start",
            "Enable startup at login and request recording now",
        ))
        .subcommand(cmd("stop", "Stop recording and disable startup at login"))
        .subcommand(cmd(
            "restart",
            "Restart background recording and reload settings",
        ))
        .subcommand(cmd(
            "uninstall",
            "Remove startup at login and retain recordings",
        ))
        .subcommand(
            cmd(
                "status",
                "Show service states, versions and recording paths",
            )
            .arg(flag("details", "Include native macOS service diagnostics")),
        ),
    )
    .subcommand(["bash", "fish", "powershell", "zsh"].into_iter().fold(
        cmd("completion", "Print shell completion scripts"),
        |c, s| {
            c.subcommand(
                cmd(s, "Print a completion script")
                    .arg(flag("no-descriptions", "Omit completion descriptions")),
            )
        },
    ));
    cli_command_kit::setup(root, mode, &help_options())
        .expect("valid ambient-recorder command tree")
}
