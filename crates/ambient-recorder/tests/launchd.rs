use ambient_recorder::{
    Error, Result,
    launchd::{Action, Agent, AgentSettings},
    service_info::{Info, Server, socket_path},
};
use std::{
    fs,
    os::unix::fs::{PermissionsExt, symlink},
    path::Path,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

fn fixture(root: &Path) -> Agent {
    let script = root.join("launchctl");
    fs::write(&script, "#!/bin/sh\ncd -- \"$(dirname -- \"$0\")\" || exit 1\nprintf '%s\\n' \"$*\" >> calls\ncase $1 in\nprint) if test -f stopping; then n=$(cat stopping); if test \"$n\" -eq 0; then rm -f loaded stopping; else echo $((n-1)) > stopping; fi; fi; if test -f loaded; then printf 'native details\\n'; else exit 113; fi;;\nbootstrap) touch loaded;;\nbootout) echo 2 > stopping;;\nenable|disable|kickstart) :;;\n*) exit 9;;\nesac\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    Agent::new(
        root.join("service.plist"),
        "gui/501".into(),
        "test.ambient".into(),
        root.join("recorder & audio"),
        script,
    )
    .unwrap()
}
fn settings(root: &Path) -> AgentSettings {
    AgentSettings {
        output: root.join("audio & speech"),
        config: Some(root.join("settings.toml")),
    }
}

#[tokio::test]
async fn install_restart_preserves_options_and_waits_for_cleanup() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let a = fixture(temp.path());
    let token = CancellationToken::new();
    a.install(&settings(temp.path()), &token).await?;
    assert!(a.installed()?);
    assert!(a.loaded(&token).await?.is_some());
    let mut value = plist::Value::from_file(&a.plist).unwrap();
    let fields = value.as_dictionary_mut().unwrap();
    fields.insert("CustomSetting".into(), plist::Value::String("kept".into()));
    fields.insert(
        "Program".into(),
        plist::Value::String("/old/recorder".into()),
    );
    fields
        .get_mut("ProgramArguments")
        .unwrap()
        .as_array_mut()
        .unwrap()[0] = plist::Value::String("/old/recorder".into());
    value.to_file_xml(&a.plist).unwrap();
    a.manage(Action::Restart, &token).await?;
    let updated = plist::Value::from_file(&a.plist).unwrap();
    let fields = updated.as_dictionary().unwrap();
    assert_eq!(fields["CustomSetting"].as_string(), Some("kept"));
    assert_eq!(fields["Program"].as_string(), a.binary.to_str());
    let args = fields["ProgramArguments"].as_array().unwrap();
    assert_eq!(args[4].as_string(), settings(temp.path()).output.to_str());
    let calls = fs::read_to_string(temp.path().join("calls"))?;
    let restart = calls.split_once("bootout ").unwrap().1;
    assert!(restart.matches("print ").count() >= 4);
    assert!(restart.contains("bootstrap gui/501"));
    assert!(!calls.contains("kickstart -k"));
    assert!(a.install(&settings(temp.path()), &token).await.is_err());
    a.manage(Action::Stop, &token).await?;
    assert!(a.loaded(&token).await?.is_none());
    assert!(a.installed()?);
    a.manage(Action::Uninstall, &token).await?;
    assert!(!a.installed()?);
    assert!(settings(temp.path()).output.is_dir());
    assert!(a.manage(Action::Start, &token).await.is_err());
    Ok(())
}

#[tokio::test]
async fn status_uses_live_identity_only_when_loaded_and_saved_arguments_otherwise() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let a = fixture(temp.path());
    let token = CancellationToken::new();
    a.install(&settings(temp.path()), &token).await?;
    let info = Info {
        version: "running-7".into(),
        output_dir: "/live/audio".into(),
        config_path: String::new(),
        current_file: String::new(),
        microphone: Some(String::new()),
    };
    let mut server = Server::listen(&socket_path(&a.plist), info).await?;
    let resolve = |args: &[String]| {
        assert_eq!(&args[1..3], ["recording", "start"]);
        Ok((
            settings(temp.path()).output,
            settings(temp.path()).config.unwrap(),
        ))
    };
    let status = a.status(&token, &resolve).await?;
    assert_eq!(status.service_version, "running-7");
    assert_eq!(status.microphone, "none");
    assert_eq!(status.paths.source, "live");
    assert_eq!(status.paths.config, "none (defaults)");
    a.manage(Action::Stop, &token).await?;
    let stopped = a.status(&token, &resolve).await?;
    assert_eq!(stopped.service_version, "not running");
    assert_eq!(stopped.paths.source, "saved");
    assert_eq!(stopped.paths.current, "not running");
    a.manage(Action::Start, &token).await?;
    server.close().await?;
    let unavailable = a.status(&token, &resolve).await?;
    assert_eq!(unavailable.service_version, "unavailable");
    assert_eq!(unavailable.paths.source, "saved");
    let bad = a
        .status(&token, &|_| {
            Err(Error::Invalid("bad saved arguments".into()))
        })
        .await?;
    assert_eq!(bad.paths.source, "unavailable");
    token.cancel();
    assert!(matches!(
        a.status(&token, &resolve).await,
        Err(Error::Cancelled)
    ));
    Ok(())
}

#[tokio::test]
async fn invalid_plists_symlinks_cancellation_and_native_exit_codes_are_preserved() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let mut a = fixture(temp.path());
    let token = CancellationToken::new();
    fs::write(temp.path().join("other"), "kept")?;
    symlink(temp.path().join("other"), &a.plist)?;
    assert!(a.installed().is_err());
    fs::remove_file(&a.plist)?;
    fs::write(&a.plist, "invalid")?;
    assert!(a.refresh(&token).await.is_err());
    let mut fields = plist::Dictionary::new();
    fields.insert("ProgramArguments".into(), plist::Value::Array(vec![]));
    plist::Value::Dictionary(fields)
        .to_file_xml(&a.plist)
        .unwrap();
    assert!(a.refresh(&token).await.is_err());
    fs::remove_file(&a.plist)?;
    a.install(&settings(temp.path()), &token).await?;
    let before = fs::read(&a.plist)?;
    assert!(!a.refresh(&token).await?);
    assert_eq!(before, fs::read(&a.plist)?);
    a.control = temp.path().join("failing");
    fs::write(
        &a.control,
        "#!/bin/sh\nprintf first\nprintf second >&2\nexit 7\n",
    )?;
    fs::set_permissions(&a.control, fs::Permissions::from_mode(0o700))?;
    let error = a.loaded(&token).await.unwrap_err();
    assert!(
        matches!(error,Error::Command {ref status, ref output,..} if status.code()==Some(7) && output==b"firstsecond")
    );
    fs::write(&a.control, "#!/bin/sh\nsleep 20\n")?;
    let cancel = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(30)).await;
        cancel.cancel();
    });
    let began = Instant::now();
    assert!(matches!(a.loaded(&token).await, Err(Error::Cancelled)));
    assert!(began.elapsed() < Duration::from_secs(1));
    Ok(())
}

#[tokio::test]
async fn failed_install_preserves_configuration_and_invalid_saved_metadata_is_rejected()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let a = fixture(temp.path());
    let token = CancellationToken::new();
    fs::write(&a.control, "#!/bin/sh\nexit 8\n")?;
    let error = a.install(&settings(temp.path()), &token).await.unwrap_err();
    assert!(
        matches!(error, Error::Multiple(ref errors) if errors.iter().any(|e| matches!(e, Error::Command { status, .. } if status.code()==Some(8))))
    );
    assert!(a.installed()?);
    let original = fs::read(&a.plist)?;
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    assert!(matches!(a.refresh(&cancelled).await, Err(Error::Cancelled)));
    assert!(matches!(
        a.install(&settings(temp.path()), &cancelled).await,
        Err(Error::Cancelled)
    ));
    assert_eq!(original, fs::read(&a.plist)?);
    for replacement in [
        plist::Value::String("not a dictionary".into()),
        plist::Value::Array(vec![]),
    ] {
        replacement.to_file_xml(&a.plist)?;
        assert!(matches!(a.refresh(&token).await, Err(Error::Invalid(_))));
    }
    let mut value = a.configuration(&settings(temp.path()))?;
    value
        .as_dictionary_mut()
        .unwrap()
        .insert("EnvironmentVariables".into(), "invalid".into());
    value.to_file_xml(&a.plist)?;
    assert!(matches!(a.refresh(&token).await, Err(Error::Invalid(_))));
    assert!(
        a.configuration(&AgentSettings {
            output: "relative".into(),
            config: None
        })
        .is_err()
    );
    use std::os::unix::ffi::OsStringExt;
    let invalid = AgentSettings {
        output: temp
            .path()
            .join(std::ffi::OsString::from_vec(vec![b'x', 255])),
        config: None,
    };
    assert!(a.configuration(&invalid).is_err());
    Ok(())
}

#[tokio::test]
async fn live_partial_identity_retains_saved_paths_and_descendant_output_has_a_deadline()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let a = fixture(temp.path());
    let token = CancellationToken::new();
    a.install(&settings(temp.path()), &token).await?;
    let mut server = Server::listen(
        &socket_path(&a.plist),
        Info {
            version: "live-partial".into(),
            output_dir: String::new(),
            config_path: String::new(),
            current_file: String::new(),
            microphone: None,
        },
    )
    .await?;
    let resolve = |_: &[String]| {
        Ok((
            settings(temp.path()).output,
            settings(temp.path()).config.unwrap(),
        ))
    };
    let status = a.status(&token, &resolve).await?;
    assert_eq!(status.service_version, "live-partial");
    assert_eq!(status.microphone, "unavailable");
    assert_eq!(status.paths.source, "saved");
    assert_eq!(status.diagnostics, b"native details\n");
    let relative = a
        .status(&token, &|_| Ok(("relative".into(), "/absolute".into())))
        .await?;
    assert_eq!(relative.paths.source, "unavailable");
    server.close().await?;
    fs::write(&a.control, "#!/bin/sh\nsleep 20 &\nexit 0\n")?;
    let began = Instant::now();
    assert!(
        matches!(a.loaded(&token).await, Err(Error::Io(e)) if e.kind()==std::io::ErrorKind::TimedOut)
    );
    assert!(began.elapsed() < Duration::from_secs(3));
    Ok(())
}
