use ambient_recorder_cli::{cli, prepare};
use cli_command_kit::Mode;
use std::{
    io::{self, Write},
    process::Command,
};
struct Broken;
impl Write for Broken {
    fn write(&mut self, _: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("closed output"))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[test]
fn offline_metadata_validates_all_help_paths_and_verbatim_skill() {
    fn visit(c: &cli_command_kit::clap::Command, path: Vec<String>, paths: &mut Vec<Vec<String>>) {
        paths.push(path.clone());
        for child in c.get_subcommands() {
            let mut p = path.clone();
            p.push(child.get_name().into());
            visit(child, p, paths)
        }
    }
    for mode in [Mode::Human, Mode::Agent] {
        let mut root = cli::command(mode);
        root.build();
        let mut paths = Vec::new();
        visit(&root, vec!["ambient-recorder".into()], &mut paths);
        for mut path in paths {
            if !path.iter().any(|s| s == "help") {
                path.push("--help".into());
            }
            let mut out = Vec::new();
            assert!(
                prepare(path.clone(), mode, &mut out).unwrap().is_none(),
                "{path:?}"
            );
            let text = String::from_utf8(out).unwrap();
            if mode == Mode::Agent {
                assert!(
                    text.starts_with("ALERT: Agents must read `AGENT=1 ambient-recorder skill`"),
                    "{path:?}: {text}"
                );
            }
            assert!(text.to_lowercase().contains("usage:"), "{path:?}");
        }
        for args in [
            vec!["ambient-recorder"],
            vec!["ambient-recorder", "recording"],
            vec!["ambient-recorder", "help", "service"],
        ] {
            assert!(prepare(args, mode, &mut Vec::new()).unwrap().is_none());
        }
        let mut skill = Vec::new();
        assert!(
            prepare(["ambient-recorder", "skill"], mode, &mut skill)
                .unwrap()
                .is_none()
        );
        assert_eq!(skill, cli::SKILL.as_bytes());
        let mut out = Vec::new();
        assert!(
            prepare(["ambient-recorder", "--version"], mode, &mut out)
                .unwrap()
                .is_none()
        );
        assert_eq!(out, b"0.1.0\n");
        for args in [
            vec!["ambient-recorder", "skill", "extra"],
            vec!["ambient-recorder", "recording", "start", "extra"],
            vec![
                "ambient-recorder",
                "recording",
                "start",
                "--bitrate",
                "5999",
            ],
            vec![
                "ambient-recorder",
                "recording",
                "start",
                "--complexity",
                "11",
            ],
            vec![
                "ambient-recorder",
                "recording",
                "start",
                "--sync-interval",
                "0s",
            ],
            vec!["ambient-recorder", "missing"],
        ] {
            assert!(prepare(args, mode, &mut Vec::new()).is_err());
        }
        assert!(prepare(["ambient-recorder", "skill"], mode, &mut Broken).is_err());
        assert!(prepare(["ambient-recorder", "--help"], mode, &mut Broken).is_err());
        assert!(prepare(["ambient-recorder", "--version"], mode, &mut Broken).is_err());
    }
}
#[test]
fn skill_documents_the_built_command_tree() {
    let missing = cli_command_kit::skill_omissions(
        &cli::command(Mode::Agent),
        &cli::help_options(),
        cli::SKILL,
    );
    assert!(missing.is_empty(), "{missing:?}");
}
#[test]
fn executable_metadata_needs_no_configuration_and_errors_are_printed_once() {
    for args in [
        vec!["--version"],
        vec!["--help"],
        vec!["skill"],
        vec!["recording"],
        vec!["help", "service", "status"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_ambient-recorder"))
            .args(args)
            .env("XDG_CONFIG_HOME", "relative-invalid")
            .env("AGENT", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
        assert!(!out.stdout.is_empty());
    }
    for args in [vec!["unknown"], vec!["config", "print-file"]] {
        let out = Command::new(env!("CARGO_BIN_EXE_ambient-recorder"))
            .args(args)
            .env("XDG_CONFIG_HOME", "relative-invalid")
            .env("AGENT", "1")
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        assert_eq!(
            String::from_utf8_lossy(&out.stderr).matches("ERR:").count(),
            1
        );
    }
    for args in [
        vec!["completion", "fish"],
        vec!["completion", "zsh", "--no-descriptions"],
    ] {
        let out = Command::new(env!("CARGO_BIN_EXE_ambient-recorder"))
            .args(args)
            .env("XDG_CONFIG_HOME", "relative-invalid")
            .env("AGENT", "0")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.stderr.is_empty());
    }
    let temp = tempfile::tempdir().unwrap();
    for mode in ["0", "1"] {
        let out = Command::new(env!("CARGO_BIN_EXE_ambient-recorder"))
            .args(["recording", "print-dir"])
            .env("XDG_CONFIG_HOME", temp.path().join("config"))
            .env("XDG_DATA_HOME", temp.path().join("data"))
            .env("AGENT", mode)
            .env("NO_COLOR", "1")
            .env("TERM", "dumb")
            .env("AMBIENT_RECORDER_SERVICE_SOCKET", "")
            .output()
            .unwrap();
        assert!(out.status.success());
        assert!(String::from_utf8_lossy(&out.stdout).contains("/data/ambient-recorder"));
        assert!(out.stderr.is_empty());
        let out = Command::new(env!("CARGO_BIN_EXE_ambient-recorder"))
            .args(["config", "print-file"])
            .env("XDG_CONFIG_HOME", temp.path().join("missing"))
            .env("AGENT", mode)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        assert!(out.stdout.is_empty());
        let text = String::from_utf8_lossy(&out.stderr);
        assert!(text.starts_with(if mode == "1" { "ERR:" } else { "[ERROR]" }));
    }
}
