use ambient_recorder::{
    Error, Result,
    capture::Native,
    config::Paths,
    launchd::Agent,
    service_info::{Info, Server, socket_path},
};
use ambient_recorder_cli::App;
use cli_command_kit::Mode;
use std::{fs, os::unix::fs::PermissionsExt, path::Path};
use tokio_util::sync::CancellationToken;
fn fixture(root: &Path, mode: Mode) -> App {
    let script = root.join("launchctl");
    fs::write(&script,"#!/bin/sh\ncd -- \"$(dirname -- \"$0\")\" || exit 1\nprintf '%s\\n' \"$*\" >> calls\ncase $1 in\nprint) test -f loaded || exit 113; printf 'native details\\n';;\nbootstrap) touch loaded;;\nbootout) rm -f loaded;;\nenable|disable|kickstart) :;;\n*) exit 9;;\nesac\n").unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700)).unwrap();
    let binary = root.join("recorder & audio");
    fs::write(&binary, "").unwrap();
    let mut app = App::new(
        Paths::new(root.into(), None, None).unwrap(),
        root.into(),
        binary.clone(),
        mode,
    );
    app.service = Some(
        Agent::new(
            root.join("service.plist"),
            "gui/501".into(),
            "test.ambient".into(),
            binary,
            script,
        )
        .unwrap(),
    );
    app
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn actual_service_dispatch_pins_paths_and_preserves_files_across_actions() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let token = CancellationToken::new();
    let mut app = fixture(root, Mode::Agent);
    let mut out = Vec::new();
    app.execute(
        ["ambient-recorder", "service", "print"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    let xml = out.clone();
    let value = plist::Value::from_reader_xml(&xml[..])?;
    let fields = value.as_dictionary().unwrap();
    assert_eq!(
        fields["ProgramArguments"].as_array().unwrap()[4].as_string(),
        app.paths.output().to_str()
    );
    assert!(!root.join("calls").exists());
    assert!(!app.paths.config().exists());
    app.mode = Mode::Human;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "print"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert_eq!(out, xml);
    app.mode = Mode::Agent;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "install"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("service=install plist=\"~/service.plist\""));
    assert!(app.paths.config().exists());
    fs::write(
        app.paths.config(),
        "invalid current TOML must not affect saved operations",
    )?;
    for action in [
        "status",
        "stop",
        "status",
        "start",
        "restart",
        "uninstall",
        "uninstall",
    ] {
        out.clear();
        app.execute(
            ["ambient-recorder", "service", action],
            &mut out,
            None,
            &token,
            &mut Native,
        )
        .await?;
        if action == "status" {
            assert!(String::from_utf8_lossy(&out).contains("paths_source=\"saved\""));
            assert!(
                String::from_utf8_lossy(&out)
                    .contains("config_path=\"~/.config/ambient-recorder/config.toml\"")
            );
        }
    }
    assert!(app.paths.config().exists());
    assert!(app.paths.output().exists());
    assert!(!app.service.as_ref().unwrap().plist.exists());
    assert!(
        app.execute(
            ["ambient-recorder", "service", "start"],
            &mut Vec::new(),
            None,
            &token,
            &mut Native
        )
        .await
        .is_err()
    );
    let calls = fs::read_to_string(root.join("calls"))?;
    assert!(calls.contains("bootout"));
    assert!(calls.contains("disable"));
    assert!(!calls.contains("kickstart -k"));
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn live_status_overrides_saved_parser_and_details_render_in_both_modes() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let token = CancellationToken::new();
    let mut app = fixture(temp.path(), Mode::Human);
    let mut out = Vec::new();
    app.execute(
        ["ambient-recorder", "service", "install"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    let agent = app.service.as_ref().unwrap().clone();
    let mut server = Server::listen(
        &socket_path(&agent.plist),
        Info {
            version: "running-7".into(),
            output_dir: temp.path().join("live audio").display().to_string(),
            config_path: String::new(),
            current_file: temp.path().join("now.opus").display().to_string(),
            microphone: Some("Fixture USB".into()),
        },
    )
    .await?;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "status", "--details"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("Running service version"));
    assert!(s.contains("running-7"));
    assert!(s.contains("~/live audio"));
    assert!(s.ends_with("native details\n"));
    app.mode = Mode::Agent;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "status", "--details=false"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    let s = String::from_utf8_lossy(&out);
    assert!(s.contains("paths_source=\"live\""));
    assert!(!s.contains("native details"));
    server.close().await?;
    let mut value = plist::Value::from_file(&agent.plist)?;
    let args = value
        .as_dictionary_mut()
        .unwrap()
        .get_mut("ProgramArguments")
        .unwrap()
        .as_array_mut()
        .unwrap();
    args.push("unexpected".into());
    value.to_file_xml(&agent.plist)?;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "status"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("paths_source=\"unavailable\""));
    let fields = value.as_dictionary_mut().unwrap();
    fields.insert(
        "ProgramArguments".into(),
        plist::Value::Array(vec![
            agent.binary.display().to_string().into(),
            "recording".into(),
            "start".into(),
            "--output".into(),
            "relative".into(),
            "--config".into(),
            temp.path().join("config.toml").display().to_string().into(),
        ]),
    );
    value.to_file_xml(&agent.plist)?;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "status"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("paths_source=\"unavailable\""));
    // The normal preview path obtains the real user ID without contacting launchd.
    app.service = None;
    out.clear();
    app.execute(
        ["ambient-recorder", "service", "print"],
        &mut out,
        None,
        &token,
        &mut Native,
    )
    .await?;
    assert!(plist::Value::from_reader_xml(&out[..]).is_ok());
    assert!(matches!(
        app.execute(
            ["ambient-recorder", "service", "status"],
            &mut Vec::new(),
            None,
            &token,
            &mut Native
        )
        .await,
        Err(Error::UnsupportedPlatform) | Err(Error::Invalid(_)) | Err(Error::Io(_))
    ));
    Ok(())
}
