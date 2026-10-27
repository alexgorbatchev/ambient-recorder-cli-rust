use ambient_recorder::{
    Error, Result,
    capture::{Acquired, Backend, Monitor, Session, Timestamp},
    config::{Paths, Settings},
    microphone::Device,
    recording::{RunConfig, Timing, run},
};
use chrono::{Datelike, Utc};
use std::{
    collections::VecDeque,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
#[derive(Clone, Default)]
struct Log(Arc<Mutex<Vec<u8>>>, bool);
impl std::io::Write for Log {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        if self.1
            && b.windows(b"Recording segment opened".len())
                .any(|part| part == b"Recording segment opened")
        {
            std::thread::sleep(Duration::from_millis(30));
        }
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
enum Read {
    Empty,
    Frames(usize, f64),
    Cancel(usize),
    Fail,
}
struct Plan {
    rate: f64,
    reads: VecDeque<Read>,
    stop_error: bool,
    close_error: bool,
    changed: bool,
    clock_error: bool,
}
impl Plan {
    fn new(reads: Vec<Read>) -> Self {
        Self {
            rate: 48_000.,
            reads: reads.into(),
            stop_error: false,
            close_error: false,
            changed: false,
            clock_error: false,
        }
    }
}
struct Input {
    plan: Plan,
    device: Device,
    token: CancellationToken,
    closed: Arc<Mutex<Vec<u32>>>,
}
impl Session for Input {
    fn sample_rate(&self) -> f64 {
        self.plan.rate
    }
    fn microphone(&self) -> &Device {
        &self.device
    }
    fn aggregate_id(&self) -> u32 {
        99
    }
    fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Acquired>> {
        let step = self.plan.reads.pop_front().unwrap_or(Read::Empty);
        let (frames, sample) = match step {
            Read::Empty => return Ok(None),
            Read::Fail => return Err(Error::Invalid("fixture read failure".into())),
            Read::Frames(n, t) => (n, t),
            Read::Cancel(n) => {
                self.token.cancel();
                if n == 0 {
                    return Ok(None);
                }
                (n, 999.)
            }
        };
        let valid = frames.min(pcm.len());
        pcm[..valid].fill(0.1);
        Ok(Some(Acquired {
            frames,
            time: Utc::now(),
            timestamp: Timestamp {
                host_time: Some(1),
                sample_time: Some(sample),
            },
        }))
    }
    fn changed(&self) -> bool {
        self.plan.changed
    }
    fn dropped(&self) -> u64 {
        0
    }
    fn recalibrate(&mut self) -> Result<chrono::Duration> {
        if self.plan.clock_error {
            Err(Error::Invalid("fixture clock failure".into()))
        } else {
            Ok(chrono::Duration::seconds(3))
        }
    }
    fn stop(&mut self) -> Result<()> {
        if self.plan.stop_error {
            Err(Error::Invalid("fixture stop failure".into()))
        } else {
            Ok(())
        }
    }
    fn close(&mut self) -> Result<()> {
        self.closed.lock().unwrap().push(self.device.id);
        if self.plan.close_error {
            Err(Error::Cleanup("fixture close failure".into()))
        } else {
            Ok(())
        }
    }
}
struct Notifications {
    fail: bool,
}
impl Monitor for Notifications {
    fn changed(&mut self) -> bool {
        false
    }
    fn close(&mut self) -> Result<()> {
        if self.fail {
            Err(Error::Invalid("fixture monitor close failure".into()))
        } else {
            Ok(())
        }
    }
}
struct BackendFixture {
    token: CancellationToken,
    devices: VecDeque<Vec<Device>>,
    plans: VecDeque<Plan>,
    closed: Arc<Mutex<Vec<u32>>>,
    monitor_error: u8,
    monitor_close: bool,
    inventory_errors: usize,
    opened: Vec<u32>,
}
impl BackendFixture {
    fn new(token: &CancellationToken, plans: Vec<Plan>) -> Self {
        Self {
            token: token.clone(),
            devices: VecDeque::from([vec![Device::new(1, "", "", true)]]),
            plans: plans.into(),
            closed: Arc::default(),
            monitor_error: 0,
            monitor_close: false,
            inventory_errors: 0,
            opened: Vec::new(),
        }
    }
}
impl Backend for BackendFixture {
    fn microphones(&mut self) -> Result<Vec<Device>> {
        if self.inventory_errors > 0 {
            self.inventory_errors -= 1;
            return Err(Error::Invalid("fixture inventory failure".into()));
        }
        if self.devices.len() > 1 {
            Ok(self.devices.pop_front().unwrap())
        } else {
            Ok(self.devices.front().unwrap().clone())
        }
    }
    fn monitor(&mut self) -> Result<Box<dyn Monitor>> {
        match self.monitor_error {
            1 => Err(Error::Invalid("fixture notifications unavailable".into())),
            2 => Err(Error::Cleanup("fixture notification cleanup".into())),
            _ => Ok(Box::new(Notifications {
                fail: self.monitor_close,
            })),
        }
    }
    fn open(&mut self, d: &Device) -> Result<Box<dyn Session>> {
        self.opened.push(d.id);
        let Some(plan) = self.plans.pop_front() else {
            self.token.cancel();
            return Err(Error::Cleanup("fixture plans exhausted".into()));
        };
        Ok(Box::new(Input {
            plan,
            device: d.clone(),
            token: self.token.clone(),
            closed: self.closed.clone(),
        }))
    }
}
fn config(root: &std::path::Path) -> RunConfig {
    let paths = Paths::new(root.into(), None, None).unwrap();
    let mut settings = Settings::defaults(&paths);
    settings.output.sync_interval = "1ms".into();
    let mut cfg = RunConfig::new(settings);
    cfg.agent = true;
    cfg.timing = Timing {
        poll: Duration::from_millis(2),
        retry_initial: Duration::from_millis(2),
        retry_max: Duration::from_millis(4),
        stalled: Duration::from_millis(12),
        watchdog: Duration::from_secs(2),
        inventory: Duration::from_millis(1),
        clock: Duration::from_millis(3),
        cooldown: Duration::from_millis(10),
        status: Duration::from_millis(3),
    };
    cfg
}
fn messages(log: &Log) -> String {
    String::from_utf8(log.0.lock().unwrap().clone()).unwrap()
}

#[test]
fn notification_inventory_retry_stall_cooldown_and_resume_are_observable() {
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    let mut backend = BackendFixture::new(
        &token,
        vec![
            Plan::new(vec![]),
            Plan::new(vec![
                Read::Frames(48, 0.),
                Read::Frames(48, 400.),
                Read::Cancel(48),
                Read::Frames(48, 500.),
            ]),
        ],
    );
    backend.monitor_error = 1;
    backend.inventory_errors = 1;
    let log = Log(Arc::default(), true);
    run(
        &mut backend,
        config(dir.path()),
        Some(Box::new(log.clone())),
        &token,
    )
    .unwrap();
    assert_eq!(backend.opened, [1, 1]);
    assert_eq!(*backend.closed.lock().unwrap(), [1, 1]);
    let text = messages(&log);
    for message in [
        "notifications unavailable",
        "inventory failure",
        "no capture frames",
        "recalibrated",
        "timeline discontinuity",
        "Recorder stopped",
    ] {
        assert!(text.contains(message), "missing {message}: {text}");
    }
}

#[test]
fn inventory_disconnect_switches_device_and_clears_previous_cooldown() {
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    let mut backend = BackendFixture::new(
        &token,
        vec![Plan::new(vec![]), Plan::new(vec![Read::Cancel(480)])],
    );
    backend.devices = VecDeque::from([
        vec![Device::new(1, "First", "one", true)],
        vec![
            Device::new(2, "Second", "two", true),
            Device::new(99, "Owned aggregate", "aggregate", false),
        ],
        vec![Device::new(2, "Second", "two", true)],
    ]);
    let log = Log::default();
    run(
        &mut backend,
        config(dir.path()),
        Some(Box::new(log.clone())),
        &token,
    )
    .unwrap();
    assert_eq!(backend.opened, [1, 2]);
    let text = messages(&log);
    for message in [
        "Microphone disconnected",
        "Microphone connected",
        "preference switch",
    ] {
        assert!(text.contains(message));
    }
    assert!(!text.contains("Owned aggregate"));
}

#[test]
fn all_capture_failures_close_owned_resources_and_retain_typed_cleanup() {
    for mode in 0..7 {
        let dir = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let mut plan = Plan::new(vec![Read::Empty]);
        plan.close_error = true;
        match mode {
            0 => plan.rate = 0.,
            1 => plan.reads = VecDeque::from([Read::Fail]),
            2 => plan.reads = VecDeque::from([Read::Frames(9000, 0.)]),
            3 => plan.changed = true,
            4 => plan.clock_error = true,
            5 => {
                plan.reads = VecDeque::from([Read::Cancel(0)]);
                plan.stop_error = true
            }
            _ => plan.reads = VecDeque::from([Read::Cancel(0), Read::Fail]),
        }
        let mut backend = BackendFixture::new(&token, vec![plan]);
        backend.monitor_close = true;
        let error = run(&mut backend, config(dir.path()), None, &token).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("fixture close failure"));
        assert!(text.contains("monitor close failure"));
        assert_eq!(*backend.closed.lock().unwrap(), [1]);
    }
    let dir = tempfile::tempdir().unwrap();
    let token = CancellationToken::new();
    let mut backend = BackendFixture::new(&token, vec![]);
    backend.monitor_error = 2;
    assert!(
        run(&mut backend, config(dir.path()), None, &token)
            .unwrap_err()
            .to_string()
            .contains("notification cleanup")
    );
    assert!(backend.opened.is_empty());
}

#[test]
fn storage_recovery_replays_pending_pcm_and_cancelled_failure_retains_error() {
    for recover in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let token = CancellationToken::new();
        let cfg = config(dir.path());
        let year = cfg
            .settings
            .output
            .dir
            .join(chrono::Local::now().year().to_string());
        std::fs::create_dir_all(year.parent().unwrap()).unwrap();
        std::fs::write(&year, b"controlled obstruction").unwrap();
        let cleanup = if recover {
            Some(std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(25));
                std::fs::remove_file(year).unwrap();
            }))
        } else {
            None
        };
        let mut backend = BackendFixture::new(
            &token,
            vec![Plan::new(if recover {
                vec![Read::Frames(480, 0.), Read::Cancel(0)]
            } else {
                vec![Read::Cancel(480)]
            })],
        );
        let log = Log::default();
        let result = run(&mut backend, cfg, Some(Box::new(log.clone())), &token);
        if let Some(thread) = cleanup {
            thread.join().unwrap();
            result.unwrap();
            assert!(messages(&log).contains("Recording segment opened"));
        } else {
            assert!(result.is_err());
        }
        assert!(messages(&log).contains("Audio storage unavailable"));
        assert_eq!(*backend.closed.lock().unwrap(), [1]);
    }
}

#[test]
fn native_capture_is_explicitly_unsupported_outside_macos() {
    #[cfg(not(target_os = "macos"))]
    {
        let mut native = ambient_recorder::capture::Native;
        assert!(matches!(
            native.microphones(),
            Err(Error::UnsupportedPlatform)
        ));
        assert!(matches!(native.monitor(), Err(Error::UnsupportedPlatform)));
        assert!(matches!(
            native.open(&Device::new(1, "Fixture", "fixture", true)),
            Err(Error::UnsupportedPlatform)
        ));
    }
    let relative = PathBuf::from("relative");
    assert!(
        ambient_recorder::launchd::Agent::new(
            relative.clone(),
            "gui/501".into(),
            "label".into(),
            relative.clone(),
            relative
        )
        .is_err()
    );
}
