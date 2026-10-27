use super::{Event, Sink, Source, Watchdog};
use crate::{
    Error, Result,
    audio::EncoderConfig,
    capture::{Backend, MAX_FRAMES, Monitor, Session},
    config::Settings,
    journal::{Journal, Level, Record},
    microphone::{Device, Preferences},
    storage::Store,
};
use microphones::device_fields;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    io::Write,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Debug)]
pub struct Timing {
    pub poll: Duration,
    pub retry_initial: Duration,
    pub retry_max: Duration,
    pub stalled: Duration,
    pub watchdog: Duration,
    pub inventory: Duration,
    pub clock: Duration,
    pub cooldown: Duration,
    pub status: Duration,
}
impl Default for Timing {
    fn default() -> Self {
        Self {
            poll: Duration::from_millis(20),
            retry_initial: Duration::from_millis(250),
            retry_max: Duration::from_secs(5),
            stalled: Duration::from_secs(5),
            watchdog: Duration::from_secs(90),
            inventory: Duration::from_secs(5),
            clock: Duration::from_secs(1),
            cooldown: Duration::from_secs(30),
            status: Duration::from_secs(30),
        }
    }
}
type CurrentHandler = Box<dyn FnMut(Option<&Path>)>;
type MicrophoneHandler = Box<dyn FnMut(&str)>;
pub struct RunConfig {
    pub settings: Settings,
    pub agent: bool,
    pub color: bool,
    pub timing: Timing,
    pub on_current: Option<CurrentHandler>,
    pub on_microphone: Option<MicrophoneHandler>,
}
impl RunConfig {
    pub fn new(settings: Settings) -> Self {
        Self {
            settings,
            agent: false,
            color: false,
            timing: Timing::default(),
            on_current: None,
            on_microphone: None,
        }
    }
}
fn fatal(error: &Error) -> bool {
    match error {
        Error::Cleanup(_) | Error::UnsupportedPlatform => true,
        Error::Multiple(errors) => errors.iter().any(fatal),
        _ => false,
    }
}
fn finish(errors: Vec<Error>) -> Result<()> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(Error::Multiple(errors))
    }
}

/// Run the native owner loop, or provide a controlled Backend for offline audio
/// integration. Native device and storage work stays on this calling thread.
pub fn run(
    backend: &mut dyn Backend,
    mut config: RunConfig,
    console: Option<Box<dyn Write>>,
    token: &CancellationToken,
) -> Result<()> {
    let sync_interval = config.settings.validate()?;
    let timing = &config.timing;
    if [
        timing.poll,
        timing.retry_initial,
        timing.retry_max,
        timing.stalled,
        timing.watchdog,
        timing.inventory,
        timing.clock,
        timing.cooldown,
        timing.status,
    ]
    .iter()
    .any(Duration::is_zero)
        || timing.retry_initial > timing.retry_max
    {
        return Err(Error::Invalid(
            "recording timing intervals must be positive and retry_initial <= retry_max".into(),
        ));
    }
    let preferences = Preferences::parse(&config.settings.input.microphones)?;
    if token.is_cancelled() {
        return Ok(());
    }
    config.settings.output.dir = std::path::absolute(&config.settings.output.dir)?;
    let watchdog = Watchdog::native(config.timing.watchdog);
    let store = Arc::new(Store::open(&config.settings.output.dir)?);
    let _lock = store.lock()?;
    let journal = Journal::new(store.clone(), console, config.agent, config.color);
    let mut sink = Sink::new(
        store,
        EncoderConfig {
            sample_rate: 48_000,
            bitrate: config.settings.output.bitrate,
            complexity: config.settings.output.complexity,
        },
    )?;
    if let Some(handler) = config.on_current.take() {
        sink.on_current(handler);
    }
    let mut runner = Runner {
        backend,
        config,
        preferences,
        sink,
        journal,
        watchdog: &watchdog,
        monitor: None,
        devices: Vec::new(),
        failures: HashMap::new(),
        refreshed: None,
        initialized: false,
        aggregate: 0,
        sync_interval,
    };
    let result = runner.execute(token);
    watchdog.pulse();
    let mut errors: Vec<_> = result.err().into_iter().collect();
    if let Err(error) = runner.sink.close() {
        errors.push(error);
    }
    runner.segment_events();
    if let Some(mut monitor) = runner.monitor.take()
        && let Err(error) = monitor.close()
    {
        errors.push(error);
    }
    if let Err(error) = runner.journal.close() {
        errors.push(error);
    }
    finish(errors)
}
struct Runner<'a> {
    backend: &'a mut dyn Backend,
    config: RunConfig,
    preferences: Preferences,
    sink: Sink,
    journal: Journal,
    watchdog: &'a Watchdog,
    monitor: Option<Box<dyn Monitor>>,
    devices: Vec<Device>,
    failures: HashMap<String, Instant>,
    refreshed: Option<Instant>,
    initialized: bool,
    aggregate: u32,
    sync_interval: Duration,
}
enum End {
    Cancelled,
    Switch,
    Stalled,
}
impl Runner<'_> {
    fn log(&mut self, level: Level, message: &str, fields: Value) {
        let record = Record::now(level, message, fields)
            .expect("internal diagnostic object has no reserved fields");
        let _ = self.journal.event(record);
    }
    fn execute(&mut self, token: &CancellationToken) -> Result<()> {
        match self.backend.monitor() {
            Ok(monitor) => self.monitor = Some(monitor),
            Err(error) if fatal(&error) => return Err(error),
            Err(error) => self.log(
                Level::Warn,
                "Microphone notifications unavailable; polling inventory",
                json!({"error":error.to_string()}),
            ),
        }
        self.log(Level::Info, "Recorder started", json!({"pid":std::process::id(),"bitrate":self.config.settings.output.bitrate,
            "complexity":self.config.settings.output.complexity,"sync_interval":self.config.settings.output.sync_interval,"microphone_preferences":self.config.settings.input.microphones}));
        let mut retry = self.config.timing.retry_initial;
        while !token.is_cancelled() {
            self.watchdog.pulse();
            match self.session(token) {
                Ok(End::Switch | End::Stalled) => {
                    retry = self.config.timing.retry_initial;
                    continue;
                }
                Ok(End::Cancelled) => break,
                Err(error) if fatal(&error) || token.is_cancelled() => {
                    self.log(
                        Level::Error,
                        "Capture session ended with an error",
                        json!({"error":error.to_string()}),
                    );
                    return Err(error);
                }
                Err(error) => self.log(
                    Level::Warn,
                    "Audio capture unavailable; retrying",
                    json!({"error":error.to_string(),"retry_in":format!("{retry:?}")}),
                ),
            }
            if !self.wait(token, retry) {
                break;
            }
            retry = (retry * 2).min(self.config.timing.retry_max);
        }
        self.log(Level::Info, "Recorder stopped by cancellation", json!({}));
        Ok(())
    }
    fn session(&mut self, token: &CancellationToken) -> Result<End> {
        self.refresh(true)?;
        let mut capture = None;
        let mut failures = Vec::new();
        for device in self.preferred() {
            match self.backend.open(&device) {
                Ok(session) => {
                    capture = Some(session);
                    break;
                }
                Err(error) if fatal(&error) => return Err(error),
                Err(error) => {
                    self.failed(&device, &error);
                    failures.push(error);
                }
            }
        }
        let Some(mut capture) = capture else {
            failures.push(Error::Invalid(
                "no microphone currently available for capture".into(),
            ));
            return Err(Error::Multiple(failures));
        };
        self.aggregate = capture.aggregate_id();
        let device = capture.microphone().clone();
        if let Some(handler) = self.config.on_microphone.as_mut() {
            handler(if device.name.is_empty() {
                "unavailable"
            } else {
                &device.name
            });
        }
        let mut fields = device_fields(&device);
        fields["sample_rate"] = json!(capture.sample_rate());
        fields["playback"] = json!("system");
        fields["output_directory"] = json!(self.config.settings.output.dir);
        self.log(Level::Info, "Capture device opened", fields);
        let mut pcm = vec![0.; MAX_FRAMES];
        let mut errors = Vec::new();
        let mut outcome = End::Cancelled;
        match Source::new(capture.sample_rate()) {
            Err(error) => {
                errors.push(error);
                if let Err(error) = capture.stop() {
                    errors.push(error);
                }
            }
            Ok(mut source) => {
                match self.consume(token, &mut *capture, &mut source, &mut pcm) {
                    Ok(end) => outcome = end,
                    Err(error) => errors.push(error),
                }
                self.watchdog.pulse();
                match capture.stop() {
                    Err(error) => errors.push(error),
                    Ok(()) => loop {
                        match capture.read(&mut pcm) {
                            Ok(Some(chunk)) if chunk.frames > 0 && chunk.frames <= pcm.len() => {
                                if let Err(error) =
                                    source.write(chunk.time, &pcm[..chunk.frames], |at, pcm| {
                                        self.persist(token, at, pcm)
                                    })
                                {
                                    errors.push(error);
                                    break;
                                }
                            }
                            Ok(None) => break,
                            Ok(Some(_)) => {
                                errors.push(Error::Invalid("invalid capture frame count".into()));
                                break;
                            }
                            Err(error) => {
                                errors.push(error);
                                break;
                            }
                        }
                    },
                }
                if let Err(error) = source.finish(|at, pcm| self.persist(token, at, pcm)) {
                    errors.push(error);
                }
            }
        }
        if let Err(error) = capture.close() {
            errors.push(error);
        }
        self.aggregate = 0;
        if let Some(handler) = self.config.on_microphone.as_mut() {
            handler("");
        }
        finish(errors)?;
        Ok(outcome)
    }
    fn consume(
        &mut self,
        token: &CancellationToken,
        capture: &mut dyn Session,
        source: &mut Source,
        pcm: &mut [f32],
    ) -> Result<End> {
        let mut last_frame = Instant::now();
        let mut last_sync = last_frame;
        let mut last_status = last_frame;
        let mut last_clock = last_frame;
        let mut previous: Option<crate::capture::Acquired> = None;
        let mut frames = 0u64;
        let mut dropped = 0;
        while !token.is_cancelled() {
            self.watchdog.pulse();
            if last_clock.elapsed() >= self.config.timing.clock {
                let shift = capture.recalibrate()?;
                if !shift.is_zero() {
                    self.log(
                        Level::Warn,
                        "Capture wall clock recalibrated",
                        json!({"clock_shift":format!("{shift}")}),
                    );
                }
                last_clock = Instant::now();
                match self.refresh(false) {
                    Err(error) => self.log(
                        Level::Warn,
                        "Microphone inventory refresh failed",
                        json!({"error":error.to_string()}),
                    ),
                    Ok(())
                        if self
                            .preferred()
                            .first()
                            .is_none_or(|d| d.id != capture.microphone().id) =>
                    {
                        self.log(
                            Level::Info,
                            "Microphone preference switch requested",
                            json!({"previous_microphone_name":capture.microphone().name}),
                        );
                        return Ok(End::Switch);
                    }
                    Ok(()) => {}
                }
            }
            let chunk = capture.read(pcm)?;
            if let Some(chunk) = chunk {
                if chunk.frames == 0 || chunk.frames > pcm.len() {
                    return Err(Error::Invalid("invalid capture frame count".into()));
                }
                last_frame = Instant::now();
                if let (Some(current), Some((sample, count))) = (
                    chunk.timestamp.sample_time,
                    previous.and_then(|p| p.timestamp.sample_time.map(|s| (s, p.frames))),
                ) {
                    let gap = current - sample - count as f64;
                    if gap.abs() > 0.5 {
                        self.log(Level::Warn, "Capture timeline discontinuity", json!({"sample_gap":gap,"host_time":chunk.timestamp.host_time,"acquired_at":chunk.time}));
                    }
                }
                previous = Some(chunk);
                source.write(chunk.time, &pcm[..chunk.frames], |at, pcm| {
                    self.persist(token, at, pcm)
                })?;
                frames += chunk.frames as u64;
            }
            let lost = capture.dropped();
            if lost != dropped {
                self.log(Level::Warn, "Capture buffer overflow", json!({"dropped_frames":lost.saturating_sub(dropped),"total_dropped_frames":lost}));
                dropped = lost;
            }
            if chunk.is_none() && capture.changed() {
                return Err(Error::Invalid(
                    "microphone or capture format changed".into(),
                ));
            }
            if chunk.is_none() && last_frame.elapsed() > self.config.timing.stalled {
                self.failed(
                    capture.microphone(),
                    &Error::Invalid("no capture frames received before timeout".into()),
                );
                return Ok(End::Stalled);
            }
            if last_sync.elapsed() >= self.sync_interval {
                let audio = self.sink.sync();
                let logs = self.journal.sync();
                let errors: Vec<_> = audio.err().into_iter().chain(logs.err()).collect();
                if !errors.is_empty() {
                    self.log(
                        Level::Warn,
                        "Storage synchronization failed",
                        json!({"error":crate::display_errors(&errors)}),
                    );
                }
                last_sync = Instant::now();
            }
            if last_status.elapsed() >= self.config.timing.status {
                self.log(Level::Debug, "Capture progress", json!({"input_frames":frames,"dropped_frames":dropped,"acquired_at":previous.map(|p|p.time)}));
                last_status = Instant::now();
            }
            if chunk.is_none() {
                self.wait(token, self.config.timing.poll);
            }
        }
        Ok(End::Cancelled)
    }
    fn persist(
        &mut self,
        token: &CancellationToken,
        at: chrono::DateTime<chrono::Utc>,
        pcm: &[f32],
    ) -> Result<()> {
        let mut retry = self.config.timing.retry_initial;
        loop {
            self.watchdog.pulse();
            match self.sink.write(at.with_timezone(&chrono::Local), pcm) {
                Ok(()) => {
                    self.segment_events();
                    return Ok(());
                }
                Err(error) => {
                    let closed = self.sink.close();
                    self.segment_events();
                    self.log(Level::Warn, "Audio storage unavailable; replaying pending block into a new segment", json!({"error":error.to_string(),"close_error":closed.err().map(|e|e.to_string()),"acquired_at":at,"replayed_frames":pcm.len(),"retry_in":format!("{retry:?}")}));
                    if !self.wait(token, retry) {
                        return Err(error);
                    }
                    retry = (retry * 2).min(self.config.timing.retry_max);
                }
            }
        }
    }
    fn segment_events(&mut self) {
        for event in self.sink.events() {
            match event {
                Event::Opened { path, acquired_at } => self.log(
                    Level::Info,
                    "Recording segment opened",
                    json!({"path":path,"acquired_at":acquired_at,"sample_rate":48000,"channels":1}),
                ),
                Event::Closed {
                    path,
                    frames,
                    error,
                } => self.log(
                    Level::Info,
                    "Recording segment closed",
                    json!({"path":path,"input_frames":frames,"error":error}),
                ),
            }
        }
    }
    fn wait(&self, token: &CancellationToken, duration: Duration) -> bool {
        let start = Instant::now();
        while !token.is_cancelled() {
            let Some(remaining) = duration.checked_sub(start.elapsed()) else {
                self.watchdog.pulse();
                return true;
            };
            std::thread::sleep(remaining.min(self.config.timing.poll));
        }
        false
    }
}
#[path = "microphones.rs"]
mod microphones;
