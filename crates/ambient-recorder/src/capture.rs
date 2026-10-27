//! Bounded, allocation-free audio callback transport and host-clock correlation.
use crate::{Error, Result};
use chrono::{DateTime, Utc};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU64, Ordering},
};

#[cfg(target_os = "macos")]
#[path = "capture_native/mod.rs"]
mod native;
#[cfg(target_os = "macos")]
pub use native::Native;
#[cfg(not(target_os = "macos"))]
pub struct Native;

#[derive(Clone, Copy, Debug)]
pub struct Acquired {
    pub frames: usize,
    pub time: DateTime<Utc>,
    pub timestamp: Timestamp,
}
pub trait Session {
    fn sample_rate(&self) -> f64;
    fn microphone(&self) -> &crate::microphone::Device;
    fn aggregate_id(&self) -> u32;
    fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Acquired>>;
    fn changed(&self) -> bool;
    fn dropped(&self) -> u64;
    fn recalibrate(&mut self) -> Result<chrono::Duration>;
    fn stop(&mut self) -> Result<()>;
    fn close(&mut self) -> Result<()>;
}
pub trait Monitor {
    fn changed(&mut self) -> bool;
    fn close(&mut self) -> Result<()>;
}
pub trait Backend {
    fn microphones(&mut self) -> Result<Vec<crate::microphone::Device>>;
    fn monitor(&mut self) -> Result<Box<dyn Monitor>>;
    fn open(&mut self, device: &crate::microphone::Device) -> Result<Box<dyn Session>>;
}
#[cfg(not(target_os = "macos"))]
impl Backend for Native {
    fn microphones(&mut self) -> Result<Vec<crate::microphone::Device>> {
        Err(Error::UnsupportedPlatform)
    }
    fn monitor(&mut self) -> Result<Box<dyn Monitor>> {
        Err(Error::UnsupportedPlatform)
    }
    fn open(&mut self, _: &crate::microphone::Device) -> Result<Box<dyn Session>> {
        Err(Error::UnsupportedPlatform)
    }
}

pub const MAX_FRAMES: usize = 8192;
pub const QUEUE_BLOCKS: usize = 256;
pub const MAX_STREAMS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timestamp {
    pub host_time: Option<u64>,
    pub sample_time: Option<f64>,
}

pub struct Buffer<'a> {
    pub channels: usize,
    pub frames: usize,
    pub samples: Option<&'a [f32]>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chunk {
    pub frames: usize,
    pub timestamp: Timestamp,
}

struct Block {
    pcm: Box<[f32; MAX_FRAMES]>,
    chunk: Chunk,
}

#[derive(Default)]
struct Health {
    dropped: AtomicU64,
    changed: AtomicBool,
}

pub struct Mixer {
    channels: usize,
    free: Consumer<Block>,
    ready: Producer<Block>,
    health: Arc<Health>,
}

pub struct Reader {
    free: Producer<Block>,
    ready: Consumer<Block>,
    pending: Option<Block>,
    health: Arc<Health>,
}

/// Allocate the complete queue before starting an audio device. The callback
/// side never grows storage, blocks, or accesses the filesystem.
pub fn queue(channels: usize) -> (Mixer, Reader) {
    let (mut free_tx, free_rx) = RingBuffer::new(QUEUE_BLOCKS);
    let (ready_tx, ready_rx) = RingBuffer::new(QUEUE_BLOCKS);
    for _ in 0..QUEUE_BLOCKS {
        let block = Block {
            pcm: vec![0.; MAX_FRAMES]
                .into_boxed_slice()
                .try_into()
                .expect("fixed block length"),
            chunk: Chunk {
                frames: 0,
                timestamp: Timestamp::default(),
            },
        };
        assert!(free_tx.push(block).is_ok());
    }
    let health = Arc::new(Health::default());
    (
        Mixer {
            channels,
            free: free_rx,
            ready: ready_tx,
            health: health.clone(),
        },
        Reader {
            free: free_tx,
            ready: ready_rx,
            pending: None,
            health,
        },
    )
}

impl Mixer {
    pub fn mark_changed(&self) {
        self.health.changed.store(true, Ordering::Release);
    }

    pub fn push(&mut self, buffers: &[Buffer<'_>], timestamp: Timestamp) -> bool {
        if self.health.changed.load(Ordering::Acquire) {
            return false;
        }
        let frames = buffers.first().map_or(0, |b| b.frames);
        let mut channels = 0usize;
        let valid = !buffers.is_empty()
            && buffers.len() <= MAX_STREAMS
            && frames <= MAX_FRAMES
            && buffers.iter().all(|b| {
                channels = channels.saturating_add(b.channels);
                b.channels > 0
                    && b.frames == frames
                    && b.samples.is_none_or(|pcm| {
                        b.frames
                            .checked_mul(b.channels)
                            .is_some_and(|count| pcm.len() == count)
                    })
            })
            && channels == self.channels
            && channels > 0;
        if !valid {
            self.mark_changed();
            return false;
        }
        if frames == 0 {
            return true;
        }
        let Ok(mut block) = self.free.pop() else {
            self.health
                .dropped
                .fetch_add(frames as u64, Ordering::Relaxed);
            return false;
        };
        block.pcm[..frames].fill(0.);
        for buffer in buffers {
            if let Some(samples) = buffer.samples {
                for (frame, mixed) in block.pcm[..frames].iter_mut().enumerate() {
                    for sample in &samples[frame * buffer.channels..(frame + 1) * buffer.channels] {
                        if sample.is_finite() {
                            *mixed += *sample / channels as f32;
                        }
                    }
                }
            }
        }
        for sample in &mut block.pcm[..frames] {
            *sample = sample.clamp(-1., 1.);
        }
        block.chunk = Chunk { frames, timestamp };
        // Both rings share exactly QUEUE_BLOCKS blocks: obtaining a free block
        // guarantees room in the ready ring, even while the consumer runs.
        assert!(self.ready.push(block).is_ok());
        true
    }
}

impl Reader {
    pub fn dropped(&self) -> u64 {
        self.health.dropped.load(Ordering::Relaxed)
    }
    pub fn changed(&self) -> bool {
        self.health.changed.load(Ordering::Acquire)
    }

    pub fn read(&mut self, pcm: &mut [f32]) -> Result<Option<Chunk>> {
        if self.pending.is_none() {
            self.pending = self.ready.pop().ok();
        }
        let Some(block) = self.pending.as_ref() else {
            return Ok(None);
        };
        if pcm.len() < block.chunk.frames {
            return Err(Error::Invalid("audio read buffer is too small".into()));
        }
        let block = self.pending.take().expect("pending block inspected above");
        pcm[..block.chunk.frames].copy_from_slice(&block.pcm[..block.chunk.frames]);
        let chunk = block.chunk;
        assert!(self.free.push(block).is_ok());
        Ok(Some(chunk))
    }
}

/// Correlates monotonically acquired host ticks with wall time. Corrections
/// greater than one second re-anchor subsequent acquisition timestamps.
pub struct Clock {
    host: u64,
    wall: DateTime<Utc>,
    nanos_per_tick: f64,
}
impl Clock {
    pub fn new(host: u64, wall: DateTime<Utc>, nanos_per_tick: f64) -> Self {
        Self {
            host,
            wall,
            nanos_per_tick,
        }
    }

    pub fn acquired(&self, host: Option<u64>, fallback: DateTime<Utc>) -> DateTime<Utc> {
        let Some(host) = host else {
            return fallback;
        };
        let nanos = (i128::from(host) - i128::from(self.host)) as f64 * self.nanos_per_tick;
        if !nanos.is_finite() || nanos < i64::MIN as f64 || nanos >= i64::MAX as f64 {
            return fallback;
        }
        self.wall
            .checked_add_signed(chrono::Duration::nanoseconds(nanos.round() as i64))
            .unwrap_or(fallback)
    }

    pub fn refresh(&mut self, host: u64, wall: DateTime<Utc>) -> Result<bool> {
        if !self.nanos_per_tick.is_finite() || self.nanos_per_tick <= 0. {
            return Err(Error::Invalid("invalid host clock scale".into()));
        }
        let delta = wall.signed_duration_since(self.acquired(Some(host), wall));
        if delta.abs() <= chrono::Duration::seconds(1) {
            return Ok(false);
        }
        self.host = host;
        self.wall = wall;
        Ok(true)
    }
}
