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
