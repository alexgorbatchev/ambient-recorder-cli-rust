use super::*;
use ambient_recorder::launchd::RecordingPaths;
#[test]
fn formatting_preserves_native_diagnostic_bytes_and_folds_every_valid_path_span() {
    let temp = tempfile::tempdir().unwrap();
    let home = temp.path();
    let agent = Agent::new(
        home.join("service.plist"),
        "gui/501".into(),
        "test.ambient".into(),
        home.join("recorder"),
        home.join("control"),
    )
    .unwrap();
    for mode in [Mode::Human, Mode::Agent] {
        let mut output = Vec::new();
        microphones(&mut output, &[], mode, home).unwrap();
        assert_eq!(output, b"No microphones available\n");
        let device = Device::new(2, "Plain input", "", false);
        output.clear();
        microphones(&mut output, &[device], mode, home).unwrap();
        assert!(String::from_utf8_lossy(&output).contains("Plain input"));
        assert!(!String::from_utf8_lossy(&output).contains("Preference:"));
        for action in ["install", "start", "stop", "restart", "uninstall"] {
            output.clear();
            service_result(&mut output, action, &agent, mode, home).unwrap();
            let text = String::from_utf8(output.clone()).unwrap();
            assert!(text.contains("~/service.plist"));
            assert!(!text.contains(&home.display().to_string()));
        }
        let mut diagnostics = format!("path={} ", home.join("before").display()).into_bytes();
        diagnostics.push(255);
        diagnostics.extend(format!(" path={} ", home.join("after").display()).as_bytes());
        diagnostics.extend([0xe2, 0x82]);
        let status = ServiceStatus {
            installed: true,
            loaded: true,
            service_version: "fixture-v7".into(),
            microphone: "Fixture".into(),
            paths: RecordingPaths {
                output: home.join("audio").display().to_string(),
                config: home.join("config.toml").display().to_string(),
                current: "none".into(),
                source: "saved".into(),
            },
            diagnostics,
        };
        output.clear();
        super::status(&mut output, &status, &agent, mode, home, true).unwrap();
        assert!(output.ends_with(b"path=~/before \xff path=~/after \xe2\x82"));
        assert!(!String::from_utf8_lossy(&output).contains(&home.display().to_string()));
        if mode == Mode::Human {
            assert!(String::from_utf8_lossy(&output).contains("Recording directory (saved)"));
        }
    }
}
