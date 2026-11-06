use ambient_recorder::{
    config::{Overrides, Paths, Settings, parse_interval},
    microphone::{Device, Preferences},
};
use std::{path::PathBuf, time::Duration};

#[test]
fn overlays_are_strict_but_validated_after_explicit_flags() {
    let root = tempfile::tempdir().unwrap();
    let paths = Paths::new(root.path().to_path_buf(), None, None).unwrap();
    let file = root.path().join("settings.toml");
    std::fs::write(
        &file,
        "[output]\nbitrate=1\ncomplexity=8\n[input]\nmicrophones=['USB*','!Virtual*']",
    )
    .unwrap();
    let (mut cfg, loaded) = Settings::load(&paths, Some(&file)).unwrap();
    assert_eq!(loaded, Some(file.clone()));
    assert!(cfg.validate().is_err());
    cfg.apply(Overrides {
        bitrate: Some(24000),
        ..Default::default()
    });
    assert_eq!(cfg.validate().unwrap(), Duration::from_secs(5));
    assert_eq!(cfg.output.complexity, 8);
    assert_eq!(cfg.output.dir, paths.output());
    for invalid in [
        "unknown=1",
        "[output]\nunknown=1",
        "[input]\nmicrophones='*'",
        "[output]\nbitrate='32000'",
    ] {
        std::fs::write(&file, invalid).unwrap();
        assert!(Settings::load(&paths, Some(&file)).is_err(), "{invalid}");
    }
    std::fs::remove_file(&file).unwrap();
    assert!(Settings::load(&paths, Some(&file)).is_err());
    assert_eq!(Settings::load(&paths, None).unwrap().1, None);
    let created = Settings::create(&paths, None).unwrap();
    assert_eq!(created, paths.config());
    assert!(Settings::create(&paths, None).is_err());
    assert!(Settings::load(&paths, None).unwrap().0.validate().is_ok());
    assert!(Paths::new(PathBuf::new(), None, None).is_err());
    assert!(Paths::new(root.path().into(), Some("relative".into()), None).is_err());
    assert!(Paths::new(root.path().into(), None, Some("relative".into())).is_err());
}

#[test]
fn duration_grammar_is_go_positive_and_unicode_safe() {
    for (value, nanos) in [
        ("1m30s", 90_000_000_000),
        (".5s", 500_000_000),
        ("+3.s", 3_000_000_000),
        ("1µs2μs3ns", 3003),
        ("1.0000000000000000000000000001s", 1_000_000_000),
    ] {
        assert_eq!(parse_interval(value).unwrap().as_nanos(), nanos);
    }
    for value in [
        "",
        "0",
        "0s",
        "-1s",
        "1 s",
        "1day",
        "1e3s",
        "1s+2s",
        "9223372036854775808ns",
        "🐈",
        "1µ",
        ".s",
    ] {
        assert!(parse_interval(value).is_err(), "{value}");
    }
}

#[test]
fn preferences_preserve_literals_exclusions_and_stable_priority() {
    let devices = vec![
        Device::new(3, "Virtual USB", "v", true),
        Device::new(2, "USB [Desk]\nMic", "b", false),
        Device::new(1, "Internal", "a", false),
    ];
    let p = Preferences::parse(&["USB [*".into(), "!Virtual*".into()]).unwrap();
    assert_eq!(
        p.ordered(&devices).iter().map(|d| d.id).collect::<Vec<_>>(),
        [2, 1]
    );
    let p = Preferences::parse(&["uid:b".into(), "*".into()]).unwrap();
    assert_eq!(
        p.ordered(&devices).iter().map(|d| d.id).collect::<Vec<_>>(),
        [2, 3, 1]
    );
    assert_eq!(Preferences::parse(&[]).unwrap().ordered(&devices)[0].id, 3);
    assert!(
        Preferences::parse(&["uid:b*".into(), "!*".into()])
            .unwrap()
            .ordered(&devices)
            .is_empty()
    );
    for invalid in ["", "!", "uid:", "!uid: "] {
        assert!(Preferences::parse(&[invalid.into()]).is_err());
    }
}

#[tokio::test]
async fn environment_paths_follow_process_settings() {
    if let Ok(case) = std::env::var("AMBIENT_PATH_CASE") {
        let root = PathBuf::from(std::env::var_os("AMBIENT_PATH_ROOT").unwrap());
        let result = Paths::from_environment();
        match case.as_str() {
            "missing" => assert!(result.unwrap_err().to_string().contains("HOME is required")),
            "relative" => assert!(
                result
                    .unwrap_err()
                    .to_string()
                    .contains("XDG_CONFIG_HOME must be an absolute path")
            ),
            "defaults" | "empty" | "explicit" => {
                let paths = result.unwrap();
                let (config, output) = if case == "explicit" {
                    (
                        root.join("config/ambient-recorder/config.toml"),
                        root.join("data/ambient-recorder"),
                    )
                } else {
                    (
                        root.join("home/.config/ambient-recorder/config.toml"),
                        root.join("home/.local/share/ambient-recorder"),
                    )
                };
                assert_eq!(paths.config(), config);
                assert_eq!(paths.output(), output);
                assert_eq!(paths.resolve_config(None).unwrap(), config);
                assert_eq!(
                    paths
                        .resolve_config(Some(std::path::Path::new("")))
                        .unwrap(),
                    config
                );
                assert_eq!(
                    paths
                        .resolve_config(Some(std::path::Path::new("custom.toml")))
                        .unwrap(),
                    root.join("custom.toml")
                );
            }
            _ => panic!("unknown path fixture case"),
        }
        std::fs::write(root.join("checked"), case).unwrap();
        return;
    }
    for case in ["defaults", "empty", "explicit", "missing", "relative"] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let mut command = tokio::process::Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "environment_paths_follow_process_settings",
                "--nocapture",
            ])
            .current_dir(&root)
            .env("AMBIENT_PATH_CASE", case)
            .env("AMBIENT_PATH_ROOT", &root)
            .env_remove("HOME")
            .env_remove("XDG_CONFIG_HOME")
            .env_remove("XDG_DATA_HOME")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        if case != "missing" && case != "explicit" {
            command.env("HOME", root.join("home"));
        }
        match case {
            "empty" => {
                command.env("XDG_CONFIG_HOME", "").env("XDG_DATA_HOME", "");
            }
            "explicit" => {
                command
                    .env("XDG_CONFIG_HOME", root.join("config"))
                    .env("XDG_DATA_HOME", root.join("data"));
            }
            "relative" => {
                command.env("XDG_CONFIG_HOME", "relative");
            }
            _ => {}
        }
        let mut child = command.spawn().unwrap();
        let result = tokio::time::timeout(Duration::from_secs(5), child.wait()).await;
        if result.is_err() {
            if let Some(pid) = child.id() {
                // SAFETY: this PID belongs to our still-owned, unreaped child.
                unsafe {
                    libc::kill(pid as i32, libc::SIGTERM);
                }
            }
            if tokio::time::timeout(Duration::from_secs(1), child.wait())
                .await
                .is_err()
            {
                child.kill().await.unwrap();
                child.wait().await.unwrap();
            }
        }
        assert!(
            result.expect("path fixture timed out").unwrap().success(),
            "{case}"
        );
        assert_eq!(std::fs::read_to_string(root.join("checked")).unwrap(), case);
    }
}
