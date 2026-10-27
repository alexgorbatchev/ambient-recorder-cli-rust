use crate::Result;
use clap_complete::aot::{Generator, Shell};
use cli_command_kit::{Mode, clap::Command};
use std::io::Write;
pub(super) fn write(shell: &str, no_descriptions: bool, out: &mut dyn Write) -> Result<()> {
    let generator = match shell {
        "bash" => Shell::Bash,
        "fish" => Shell::Fish,
        "powershell" => Shell::PowerShell,
        "zsh" => Shell::Zsh,
        _ => unreachable!("parsed completion shell"),
    };
    let mut command = crate::cli::command(Mode::Human);
    if no_descriptions {
        strip(&mut command);
    }
    command.set_bin_name("ambient-recorder");
    command.build();
    // Native Fish generation assumes an infallible sink; isolate it in memory.
    let mut script = Vec::new();
    generator.try_generate(&command, &mut script)?;
    out.write_all(&script)?;
    Ok(())
}
fn strip(command: &mut Command) {
    *command = command
        .clone()
        .about("")
        .long_about("")
        .mut_args(|a| a.help("").long_help(""));
    for child in command.get_subcommands_mut() {
        strip(child)
    }
}
