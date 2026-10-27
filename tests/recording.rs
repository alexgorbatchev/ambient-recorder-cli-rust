use ambient_recorder::{
    Error, Result,
    capture::{Acquired, Backend, Monitor, Session, Timestamp},
    config::Paths,
    microphone::Device,
    service_info,
    storage::Store,
};
use ambient_recorder_cli::App;
use cli_command_kit::Mode;
use std::{
    fs,
    io::{self, Write},
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio_util::sync::CancellationToken;
struct Fixture {
    token: CancellationToken,
    device: Device,
    socket: Option<PathBuf>,
    observed: Arc<Mutex<Vec<service_info::Info>>>,
    fail: bool,
}
struct Input {
    token: CancellationToken,
    device: Device,
    socket: Option<PathBuf>,
    observed: Arc<Mutex<Vec<service_info::Info>>>,
    read: usize,
}
struct Notifications;
impl Monitor for Notifications {
    fn changed(&mut self) -> bool {
        false
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
impl Backend for Fixture {
    fn microphones(&mut self) -> Result<Vec<Device>> {
        if self.fail {
            Err(Error::Invalid("controlled inventory failure".into()))
        } else {
            Ok(vec![self.device.clone()])
        }
    }
    fn monitor(&mut self) -> Result<Box<dyn Monitor>> {
        if self.fail {
            Err(Error::Invalid("controlled monitor failure".into()))
        } else {
            Ok(Box::new(Notifications))
        }
    }
    fn open(&mut self, d: &Device) -> Result<Box<dyn Session>> {
        Ok(Box::new(Input {
            token: self.token.clone(),
            device: d.clone(),
            socket: self.socket.clone(),
            observed: self.observed.clone(),
            read: 0,
        }))
    }
}
impl Session for Input {
    fn sample_rate(&self) -> f64 {
        48000.
    }
    fn microphone(&self) -> &Device {
        &self.device
    }
    fn aggregate_id(&self) -> u32 {
        77
    }
    fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Acquired>> {
        if self.read == 1 {
            if let Some(socket) = self.socket.clone() {
                let (tx, rx) = std::sync::mpsc::channel();
                tokio::runtime::Handle::current().spawn(async move {
                    let _ = tx.send(service_info::query(&socket, &CancellationToken::new()).await);
                });
                let info = rx.recv_timeout(Duration::from_secs(2)).unwrap()?;
                self.observed.lock().unwrap().push(info);
            }
            self.token.cancel();
            self.read += 1;
            return Ok(None);
        }
        if self.read > 1 {
            return Ok(None);
        }
        pcm[..4800].fill(0.125);
        self.read += 1;
        Ok(Some(Acquired {
            frames: 4800,
            time: chrono::Utc::now(),
            timestamp: Timestamp {
                host_time: None,
                sample_time: Some(0.),
            },
        }))
    }
    fn changed(&self) -> bool {
        false
    }
    fn dropped(&self) -> u64 {
        0
    }
    fn recalibrate(&mut self) -> Result<chrono::Duration> {
        Ok(chrono::Duration::zero())
    }
    fn stop(&mut self) -> Result<()> {
        Ok(())
    }
    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}
fn fixture(token: &CancellationToken, socket: Option<PathBuf>) -> Fixture {
    let mut device = Device::new(1, "Fixture USB", "uid-example", true);
    device.transport = "USB".into();
    device.terminal = "Microphone".into();
    device.manufacturer = "Example".into();
    device.model = "Local fixture".into();
    Fixture {
        token: token.clone(),
        device,
        socket,
        observed: Arc::default(),
        fail: false,
    }
}
struct Console(Arc<Mutex<Vec<u8>>>);
impl Write for Console {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn controlled_recording_publishes_native_identity_and_cleans_socket_and_segment() -> Result<()>
{
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let paths = Paths::new(root.into(), None, None)?;
    let token = CancellationToken::new();
    let socket = root.join("record.sock");
    let mut backend = fixture(&token, Some(socket.clone()));
    let observed = backend.observed.clone();
    let mut app = App::new(
        paths.clone(),
        root.into(),
        root.join("recorder"),
        Mode::Agent,
    );
    app.service_socket = Some(socket.clone());
    fs::create_dir_all(paths.config().parent().unwrap())?;
    fs::write(
        paths.config(),
        "[output]\nbitrate=1\ncomplexity=99\nsync_interval='invalid'\n",
    )?;
    let output = root.join("controlled audio");
    let console: Arc<Mutex<Vec<u8>>> = Arc::default();
    let mut out = Vec::new();
    app.execute(
        [
            "ambient-recorder",
            "recording",
            "start",
            "--output",
            output.to_str().unwrap(),
            "--bitrate",
            "64000",
            "--complexity",
            "3",
            "--sync-interval",
            "250ms",
        ],
        &mut out,
        Some(Box::new(Console(console.clone()))),
        &token,
        &mut backend,
    )
    .await?;
    assert!(out.is_empty());
    assert!(!socket.exists());
    assert!(ambient_recorder::storage::current_file(&output).is_err());
    let info = observed.lock().unwrap();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].microphone.as_deref(), Some("Fixture USB"));
    assert_eq!(info[0].output_dir, output.display().to_string());
    assert_eq!(info[0].config_path, paths.config().display().to_string());
    assert!(!info[0].current_file.is_empty());
    let audio = PathBuf::from(&info[0].current_file);
    assert!(fs::read(audio)?.starts_with(b"OggS"));
    assert!(!console.lock().unwrap().is_empty());
    drop(info);
    let store = Store::open(&output)?;
    let _lock = store.lock()?;
    Ok(())
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn capture_start_failure_and_cancelled_defaults_release_private_identity() -> Result<()> {
    let temp = tempfile::tempdir()?;
    let root = temp.path();
    let paths = Paths::new(root.into(), None, None)?;
    let token = CancellationToken::new();
    let mut backend = fixture(&token, None);
    backend.fail = true;
    let mut app = App::new(paths, root.into(), root.join("recorder"), Mode::Human);
    let socket = root.join("startup.sock");
    app.service_socket = Some(socket.clone());
    let store = Store::open(app.paths.output())?;
    let _lock = store.lock()?;
    assert!(
        app.execute(
            ["ambient-recorder", "recording", "start"],
            &mut Vec::new(),
            None,
            &token,
            &mut backend
        )
        .await
        .is_err()
    );
    assert!(!socket.exists());
    token.cancel();
    app.execute(
        ["ambient-recorder", "recording", "start"],
        &mut Vec::new(),
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert!(!socket.exists());
    fs::write(&socket, "retain unrelated file")?;
    assert!(
        app.execute(
            ["ambient-recorder", "recording", "start"],
            &mut Vec::new(),
            None,
            &CancellationToken::new(),
            &mut backend
        )
        .await
        .is_err()
    );
    assert_eq!(fs::read(&socket)?, b"retain unrelated file");
    Ok(())
}
#[tokio::test]
async fn microphone_views_and_current_file_queries_use_real_inventory_and_lock_ownership()
-> Result<()> {
    let temp = tempfile::tempdir()?;
    let paths = Paths::new(temp.path().into(), None, None)?;
    let token = CancellationToken::new();
    let mut backend = fixture(&token, None);
    let mut app = App::new(
        paths.clone(),
        temp.path().into(),
        temp.path().join("recorder"),
        Mode::Human,
    );
    let mut out = Vec::new();
    app.execute(
        ["ambient-recorder", "microphone", "list"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("[system default]"));
    assert!(String::from_utf8_lossy(&out).contains("Preference: \"uid:uid-example\""));
    app.mode = Mode::Agent;
    out.clear();
    app.execute(
        ["ambient-recorder", "microphone", "list"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).contains("device_id=1 default=true transport=\"USB\""));
    let store = Store::open(paths.output())?;
    let lock = store.lock()?;
    let segment = store.create_segment(chrono::Utc::now())?;
    store.mark_current(&segment)?;
    out.clear();
    app.execute(
        ["ambient-recorder", "recording", "print-file"],
        &mut out,
        None,
        &token,
        &mut backend,
    )
    .await?;
    assert!(String::from_utf8_lossy(&out).starts_with("~/.local/share/ambient-recorder/"));
    drop(segment);
    drop(lock);
    out.clear();
    assert!(
        app.execute(
            ["ambient-recorder", "recording", "print-file"],
            &mut out,
            None,
            &token,
            &mut backend
        )
        .await
        .is_err()
    );
    assert!(out.is_empty());
    let missing = temp.path().join("missing.toml");
    let err = app
        .execute(
            [
                "ambient-recorder",
                "--config",
                missing.to_str().unwrap(),
                "recording",
                "start",
                "--output=",
            ],
            &mut out,
            None,
            &token,
            &mut backend,
        )
        .await
        .unwrap_err();
    assert!(matches!(err,Error::Invalid(s) if s=="--output must be nonempty"));
    Ok(())
}
#[tokio::test]
async fn completion_outputs_native_scripts_and_propagates_closed_output() -> Result<()> {
    struct Broken;
    impl Write for Broken {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("closed"))
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let temp = tempfile::tempdir()?;
    let token = CancellationToken::new();
    let mut app = App::new(
        Paths::new(temp.path().into(), None, None)?,
        temp.path().into(),
        temp.path().join("recorder"),
        Mode::Agent,
    );
    let mut backend = fixture(&token, None);
    for shell in ["bash", "fish", "powershell", "zsh"] {
        let mut a = Vec::new();
        let mut b = Vec::new();
        app.execute(
            ["ambient-recorder", "completion", shell],
            &mut a,
            None,
            &token,
            &mut backend,
        )
        .await?;
        app.execute(
            ["ambient-recorder", "completion", shell, "--no-descriptions"],
            &mut b,
            None,
            &token,
            &mut backend,
        )
        .await?;
        assert!(String::from_utf8_lossy(&a).contains("ambient-recorder"));
        assert!(!b.is_empty());
        assert!(
            app.execute(
                ["ambient-recorder", "completion", shell],
                &mut Broken,
                None,
                &token,
                &mut backend
            )
            .await
            .is_err()
        );
    }
    Ok(())
}
