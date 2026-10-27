use ambient_recorder::{
    audio::EncoderConfig,
    recording::Sink,
    storage::{Store, current_file},
};
use chrono::{TimeZone, Timelike};
use std::sync::{Arc, Mutex};

fn decoded_frames(path: &std::path::Path) -> usize {
    let output = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args(["-f", "f32le", "-acodec", "pcm_f32le", "-"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout.len() / 4
}
#[test]
fn acquired_hour_rotation_owns_distinct_valid_opus_streams_and_current_identity() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let _lock = store.lock().unwrap();
    let paths = Arc::new(Mutex::new(Vec::new()));
    let mut sink = Sink::new(store.clone(), EncoderConfig::default()).unwrap();
    let observed = paths.clone();
    sink.on_current(move |path| {
        observed
            .lock()
            .unwrap()
            .push(path.map(std::path::Path::to_path_buf))
    });
    let at = chrono::Utc
        .with_ymd_and_hms(2026, 10, 14, 1, 59, 59)
        .unwrap()
        .with_nanosecond(500_000_000)
        .unwrap();
    sink.write(at, &vec![0.1; 48_000]).unwrap();
    assert!(current_file(dir.path()).unwrap().exists());
    sink.sync().unwrap();
    sink.close().unwrap();
    sink.close().unwrap();
    assert!(current_file(dir.path()).is_err());
    let paths = paths.lock().unwrap();
    assert_eq!(paths.len(), 4);
    let first = paths[0].as_ref().unwrap();
    let second = paths[2].as_ref().unwrap();
    assert_ne!(first, second);
    assert_eq!(decoded_frames(first), 24_000);
    assert_eq!(decoded_frames(second), 24_000);
    assert!(paths[1].is_none() && paths[3].is_none());
}
#[test]
fn daylight_saving_repeated_hour_and_large_frame_duration_keep_acquired_boundaries() {
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(Store::open(dir.path()).unwrap());
    let paths = Arc::new(Mutex::new(Vec::new()));
    let mut sink = Sink::new(store, EncoderConfig::default()).unwrap();
    let observed = paths.clone();
    sink.on_current(move |path| {
        if let Some(path) = path {
            observed.lock().unwrap().push(path.to_path_buf());
        }
    });
    let at = chrono::Utc
        .with_ymd_and_hms(2026, 11, 1, 8, 59, 59)
        .unwrap()
        .with_nanosecond(500_000_000)
        .unwrap()
        .with_timezone(&chrono_tz::America::Los_Angeles);
    sink.write(at, &vec![0.; 48_000]).unwrap();
    sink.close().unwrap();
    let paths = paths.lock().unwrap();
    assert_eq!(paths.len(), 2);
    assert_eq!(decoded_frames(&paths[0]), 24_000);
    assert_eq!(decoded_frames(&paths[1]), 24_000);
    assert_eq!(
        ambient_recorder::recording::frame_duration(48_000 * 86_400 * 365, 48_000).unwrap(),
        chrono::Duration::days(365)
    );
    assert!(ambient_recorder::recording::frame_duration(1, 0).is_err());
}
