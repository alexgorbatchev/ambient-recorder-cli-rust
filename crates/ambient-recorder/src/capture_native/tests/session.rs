use super::*;

pub(super) fn session(input: Input, host: u64, wall: chrono::DateTime<chrono::Utc>) -> Capture {
    Capture {
        microphone: Device::new(0, "Controlled PCM", "controlled-pcm", false),
        tap: 0,
        aggregate: 0,
        io: None,
        running: false,
        watches: Vec::new(),
        callback: Some(input.callback),
        reader: Some(input.reader),
        rate: 48_000.,
        clock: Clock::new(host, wall, 1.),
        calibrated: Instant::now(),
        closed: false,
    }
}

#[test]
fn native_session_reads_acquired_pcm_and_closes_idempotently() {
    let mut input = Input::new(1);
    let mut values = [0.25, -0.5];
    let mut list = samples(&mut values, 1);
    // Host-time queries do not open an audio device or start capture.
    let host = unsafe { AudioGetCurrentHostTime() };
    let nanos = unsafe { AudioConvertHostTimeToNanos(host) };
    let wall = chrono::Utc::now();
    let mut time = AudioTimeStamp {
        mHostTime: host,
        mFlags: AudioTimeStampFlags::HostTimeValid,
        ..timestamp()
    };
    // The input list references the live values array for this synchronous call.
    unsafe { input.deliver(&mut list, &mut time) };
    let mut capture = session(input, nanos, wall);
    let mut pcm = [0.; MAX_FRAMES];
    let acquired = capture.read(&mut pcm).unwrap().unwrap();
    assert_eq!(&pcm[..acquired.frames], &values);
    assert_eq!(acquired.time, wall);
    assert_eq!(acquired.timestamp.host_time, Some(host));
    assert!(capture.read(&mut pcm).unwrap().is_none());
    assert_eq!(capture.sample_rate(), 48_000.);
    assert_eq!(capture.microphone().uid, "controlled-pcm");
    assert_eq!(capture.aggregate_id(), 0);
    assert!(!capture.changed());
    assert_eq!(capture.dropped(), 0);
    capture.stop().unwrap();
    capture.close().unwrap();
    capture.close().unwrap();
    assert!(capture.callback.is_none());
    assert!(matches!(capture.read(&mut pcm), Err(Error::Invalid(_))));
}

#[test]
fn native_session_rejects_missing_acquisition_time_and_short_output() {
    let mut input = Input::new(1);
    let mut values = [0.5];
    let mut list = samples(&mut values, 1);
    // Only the timestamp flags are missing; the sample buffer remains valid.
    unsafe { input.deliver(&mut list, &mut timestamp()) };
    let mut capture = session(input, 0, chrono::Utc::now());
    assert!(matches!(capture.read(&mut [0.; 1]), Err(Error::Invalid(_))));
    assert!(matches!(
        capture.read(&mut [0.; MAX_FRAMES]),
        Err(Error::Invalid(message)) if message.contains("host timestamp")
    ));
    capture.close().unwrap();
}

#[test]
fn native_session_reports_overflow_changes_and_recalibrates_host_time() {
    let mut input = Input::new(1);
    let mut values = [0.5];
    let mut list = samples(&mut values, 1);
    for _ in 0..super::super::super::QUEUE_BLOCKS + 3 {
        // The same owned input is copied into distinct bounded queue blocks.
        unsafe { input.deliver(&mut list, &mut timestamp()) };
    }
    let (host, wall) = host_snapshot();
    let mut capture = session(input, host, wall - chrono::Duration::seconds(30));
    assert_eq!(capture.dropped(), 3);
    capture
        .callback
        .as_ref()
        .unwrap()
        .health
        .changed
        .store(true, Ordering::Release);
    assert!(capture.changed());
    assert_eq!(capture.recalibrate().unwrap(), chrono::Duration::zero());
    capture.calibrated = Instant::now() - Duration::from_secs(2);
    let correction = capture.recalibrate().unwrap();
    assert!(correction > chrono::Duration::seconds(29));
    assert!(correction < chrono::Duration::seconds(31));
    assert_eq!(capture.recalibrate().unwrap(), chrono::Duration::zero());
    capture.close().unwrap();
    capture.reader.take();
    assert!(!capture.changed());
    assert_eq!(capture.dropped(), 0);
}

#[test]
fn native_inventory_listeners_are_owned_and_released_without_capture() {
    let mut backend = Native;
    let mut monitor = backend.monitor().unwrap();
    assert!(monitor.changed());
    // Listing HAL metadata never opens a microphone stream. Inventory may be
    // empty on a hosted runner; every returned device must have native identity.
    for device in backend.microphones().unwrap() {
        assert_ne!(device.id, 0);
        assert!(!device.uid.is_empty());
    }
    monitor.close().unwrap();
    monitor.close().unwrap();
    assert!(!monitor.changed());
    // Drop independently exercises ownership for a second live registration.
    drop(backend.monitor().unwrap());
}
