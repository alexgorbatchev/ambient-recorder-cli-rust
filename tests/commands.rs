use ambient_recorder::{capture::Native, config::Paths};
use ambient_recorder_cli::{App, Result};
use cli_command_kit::Mode;
use tokio_util::sync::CancellationToken;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn config_path_queries_preserve_existing_bytes_and_output_overrides() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = Paths::new(temp.path().to_owned(), None, None)?;
    let mut app = App::new(
        paths.clone(),
        temp.path().to_owned(),
        temp.path().join("recorder"),
        Mode::Agent,
    );
    let token = CancellationToken::new();
    let mut out = Vec::new();
    let mut backend = Native;
    assert!(
        app.execute(
            ["ambient-recorder", "config", "print-file"],
            &mut out,
            None,
            &token,
            &mut backend
        )
        .await
        .is_err()
    );
    assert!(out.is_empty());
    app.execute(
        ["ambient-recorder", "config", "init"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert_eq!(
        String::from_utf8_lossy(&out),
        "config=~/.config/ambient-recorder/config.toml\n"
    );
    out.clear();
    std::fs::write(paths.config(), "invalid TOML kept verbatim")?;
    app.execute(
        ["ambient-recorder", "config", "print-file"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert_eq!(out, b"~/.config/ambient-recorder/config.toml\n");
    out.clear();
    assert!(
        app.execute(
            ["ambient-recorder", "config", "init"],
            &mut out,
            None,
            &token,
            &mut backend
        )
        .await
        .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(paths.config())?,
        "invalid TOML kept verbatim"
    );
    assert!(
        app.execute(
            ["ambient-recorder", "recording", "print-dir"],
            &mut out,
            None,
            &token,
            &mut backend
        )
        .await
        .is_err()
    );
    std::fs::remove_file(paths.config())?;
    app.execute(
        ["ambient-recorder", "recording", "print-dir"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert_eq!(out, b"~/.local/share/ambient-recorder\n");
    Ok(())
}

#[tokio::test]
async fn explicit_paths_remain_read_only_and_output_failures_preserve_completed_creation()
-> Result<()> {
    use std::{
        io::{self, Write},
        os::unix::fs::symlink,
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
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let paths = Paths::new(root.into(), None, None)?;
    let token = CancellationToken::new();
    let mut app = App::new(paths, root.into(), root.join("recorder"), Mode::Human);
    let config = root.join("custom.toml");
    let mut out = Vec::new();
    app.execute(
        [
            "ambient-recorder",
            "--config",
            config.to_str().unwrap(),
            "config",
            "init",
        ],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert_eq!(out, b"Configuration created: ~/custom.toml\n");
    std::fs::write(&config, "[output]\ndir='relative-audio'\n")?;
    out.clear();
    app.execute(
        [
            "ambient-recorder",
            "--config",
            config.to_str().unwrap(),
            "recording",
            "print-dir",
        ],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("/relative-audio\n"));
    assert!(!std::path::Path::new("relative-audio").exists());
    let link = root.join("linked.toml");
    symlink(&config, &link)?;
    out.clear();
    app.execute(
        [
            "ambient-recorder",
            "--config",
            link.to_str().unwrap(),
            "config",
            "print-file",
        ],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert_eq!(out, b"~/linked.toml\n");
    out.clear();
    assert!(
        app.execute(
            [
                "ambient-recorder",
                "--config",
                root.to_str().unwrap(),
                "config",
                "print-file"
            ],
            &mut out,
            None,
            &token,
            &mut Native
        )
        .await
        .is_err()
    );
    assert!(out.is_empty());
    let created = root.join("closed-output.toml");
    assert!(
        app.execute(
            [
                "ambient-recorder",
                "--config",
                created.to_str().unwrap(),
                "config",
                "init"
            ],
            &mut Broken,
            None,
            &token,
            &mut Native
        )
        .await
        .is_err()
    );
    assert!(created.is_file());
    Ok(())
}
