use ambient_recorder::{
    Error, Result,
    capture::{Acquired, Backend, Monitor, Session, Timestamp},
    config::{Paths, Settings},
    microphone::Device,
    recording::{RunConfig, Timing, Watchdog, run},
};
use chrono::Utc;
use std::{
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;
struct Fixture {
    token: CancellationToken,
    closed: Arc<Mutex<Vec<String>>>,
    fail_first: bool,
    opens: usize,
}
struct Input {
    token: CancellationToken,
    closed: Arc<Mutex<Vec<String>>>,
    device: Device,
    chunks: VecDeque<usize>,
    stopped: bool,
}
struct Notifications(Arc<Mutex<Vec<String>>>);
impl Monitor for Notifications {
    fn changed(&mut self) -> bool {
        true
    }
    fn close(&mut self) -> Result<()> {
        self.0.lock().unwrap().push("monitor".into());
        Ok(())
    }
}
impl Backend for Fixture {
    fn microphones(&mut self) -> Result<Vec<Device>> {
        Ok(vec![
            Device::new(1, "Bad input", "bad", true),
            Device::new(2, "Fixture input", "fixture", false),
        ])
    }
    fn monitor(&mut self) -> Result<Box<dyn Monitor>> {
        Ok(Box::new(Notifications(self.closed.clone())))
    }
    fn open(&mut self, device: &Device) -> Result<Box<dyn Session>> {
        self.opens += 1;
        if self.fail_first && device.id == 1 {
            return Err(Error::Invalid("controlled unavailable input".into()));
        }
        Ok(Box::new(Input {
            token: self.token.clone(),
            closed: self.closed.clone(),
            device: device.clone(),
            chunks: VecDeque::from([1600, 1600, 1600]),
            stopped: false,
        }))
    }
}
impl Session for Input {
    fn sample_rate(&self) -> f64 {
        16_000.
    }
    fn microphone(&self) -> &Device {
        &self.device
    }
    fn aggregate_id(&self) -> u32 {
        99
    }
    fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Acquired>> {
        let Some(frames) = self.chunks.pop_front() else {
            if !self.stopped {
                self.token.cancel();
            }
            return Ok(None);
        };
        pcm[..frames].fill(0.1);
        Ok(Some(Acquired {
            frames,
            time: Utc::now(),
            timestamp: Timestamp {
                host_time: Some(123),
                sample_time: Some((4800 - self.chunks.len() * 1600 - 1600) as f64),
            },
        }))
    }
    fn changed(&self) -> bool {
        false
    }
    fn dropped(&self) -> u64 {
        12
    }
    fn recalibrate(&mut self) -> Result<chrono::Duration> {
        Ok(chrono::Duration::zero())
    }
    fn stop(&mut self) -> Result<()> {
        self.stopped = true;
        self.closed.lock().unwrap().push("stop".into());
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        self.closed.lock().unwrap().push("input".into());
        Ok(())
    }
}
#[test]
fn controlled_capture_retries_preferences_encodes_drains_and_releases_all_owned_resources() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().into(), None, None).unwrap();
    let mut settings = Settings::defaults(&paths);
    settings.output.dir = dir.path().join("audio");
    let token = CancellationToken::new();
    let closed = Arc::new(Mutex::new(Vec::new()));
    let mut backend = Fixture {
        token: token.clone(),
        closed: closed.clone(),
        fail_first: true,
        opens: 0,
    };
    let names = Arc::new(Mutex::new(Vec::new()));
    let recorded = names.clone();
    let mut config = RunConfig::new(settings);
    config.on_microphone = Some(Box::new(move |name| {
        recorded.lock().unwrap().push(name.to_owned())
    }));
    run(&mut backend, config, None, &token).unwrap();
    assert_eq!(backend.opens, 2);
    assert_eq!(*names.lock().unwrap(), ["Fixture input", ""]);
    assert_eq!(*closed.lock().unwrap(), ["stop", "input", "monitor"]);
    assert!(ambient_recorder::storage::current_file(dir.path().join("audio")).is_err());
    let root = dir.path().join("audio");
    let mut opus = Vec::new();
    fn visit(path: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                visit(&path, out)
            } else if path.extension().is_some_and(|e| e == "opus") {
                out.push(path)
            }
        }
    }
    visit(&root, &mut opus);
    assert_eq!(opus.len(), 1);
    let decoded = std::process::Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&opus[0])
        .args(["-f", "f32le", "-"])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert_eq!(decoded.stdout.len() / 4, 14_400);
    let _newlock = ambient_recorder::storage::Store::open(root)
        .unwrap()
        .lock()
        .unwrap();
}
#[test]
fn cancelled_or_invalid_start_does_not_open_storage_or_capture() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::new(dir.path().into(), None, None).unwrap();
    let settings = Settings::defaults(&paths);
    let output = settings.output.dir.clone();
    let token = CancellationToken::new();
    token.cancel();
    let mut backend = Fixture {
        token: token.clone(),
        closed: Arc::default(),
        fail_first: false,
        opens: 0,
    };
    run(&mut backend, RunConfig::new(settings.clone()), None, &token).unwrap();
    assert!(!output.exists());
    let mut invalid = settings;
    invalid.output.bitrate = 0;
    assert!(run(&mut backend, RunConfig::new(invalid), None, &token).is_err());
    assert_eq!(backend.opens, 0);
    let mut invalid = RunConfig::new(Settings::defaults(&paths));
    invalid.timing = Timing {
        poll: Duration::ZERO,
        ..Timing::default()
    };
    assert!(run(&mut backend, invalid, None, &token).is_err());
}
#[test]
fn watchdog_is_independent_of_a_blocked_diagnostic_and_stops_promptly() {
    let exited = Arc::new(AtomicBool::new(false));
    let flag = exited.clone();
    let (tx, rx) = std::sync::mpsc::channel();
    let started = Instant::now();
    let mut watchdog = Watchdog::new(
        Duration::from_millis(25),
        move || {
            let _ = rx.recv_timeout(Duration::from_secs(2));
        },
        move |code| {
            assert_eq!(code, 2);
            flag.store(true, Ordering::Release);
        },
    );
    while !exited.load(Ordering::Acquire) && started.elapsed() < Duration::from_secs(1) {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(exited.load(Ordering::Acquire));
    assert!(started.elapsed() < Duration::from_millis(500));
    tx.send(()).unwrap();
    watchdog.stop();
    let mut watchdog = Watchdog::new(
        Duration::from_secs(90),
        || panic!("stopped watchdog fired"),
        |_| panic!("stopped watchdog exited"),
    );
    let start = Instant::now();
    watchdog.pulse();
    watchdog.stop();
    assert!(start.elapsed() < Duration::from_millis(100));
}
