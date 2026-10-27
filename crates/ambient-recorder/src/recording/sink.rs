use super::frame_duration;
use crate::{
    Error, Result,
    audio::{Encoder, EncoderConfig},
    storage::{Segment, Store},
};
use chrono::{DateTime, TimeZone, Timelike};
use std::{
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};

#[derive(Debug)]
pub enum Event {
    Opened {
        path: PathBuf,
        acquired_at: DateTime<chrono::FixedOffset>,
    },
    Closed {
        path: PathBuf,
        frames: u64,
        error: Option<String>,
    },
}
struct Active {
    segment: Segment,
    encoder: Encoder<File>,
    hour: String,
    frames: u64,
}
type CurrentHandler = Box<dyn FnMut(Option<&Path>)>;
pub struct Sink {
    store: Arc<Store>,
    config: EncoderConfig,
    active: Option<Active>,
    current: Option<CurrentHandler>,
    events: Vec<Event>,
}
impl Sink {
    pub fn new(store: Arc<Store>, config: EncoderConfig) -> Result<Self> {
        if config.sample_rate != 48_000
            || !(6000..=128000).contains(&config.bitrate)
            || !(0..=10).contains(&config.complexity)
        {
            return Err(Error::Invalid(
                "recording requires 48000 Hz, bitrate 6000..128000 and complexity 0..10".into(),
            ));
        }
        Ok(Self {
            store,
            config,
            active: None,
            current: None,
            events: Vec::new(),
        })
    }
    pub fn on_current(&mut self, handler: impl FnMut(Option<&Path>) + 'static) {
        self.current = Some(Box::new(handler));
    }
    pub fn events(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }
    pub fn write<Tz: TimeZone>(&mut self, at: DateTime<Tz>, pcm: &[f32]) -> Result<()>
    where
        Tz::Offset: std::fmt::Display,
    {
        let mut offset = 0;
        while offset < pcm.len() {
            let start = at
                .clone()
                .checked_add_signed(frame_duration(offset as u64, 48_000)?)
                .ok_or_else(|| Error::Invalid("acquisition timestamp overflow".into()))?;
            let hour = start.format("%Y/%m/%d/%H%z").to_string();
            if self
                .active
                .as_ref()
                .is_none_or(|active| active.hour != hour)
            {
                self.close()?;
                self.open(start.clone(), hour)?;
            }
            let nanos = (3600 - u64::from(start.minute()) * 60 - u64::from(start.second()))
                * 1_000_000_000
                - u64::from(start.nanosecond());
            let count = (nanos * 48_000)
                .div_ceil(1_000_000_000)
                .min((pcm.len() - offset) as u64) as usize;
            let active = self.active.as_mut().expect("opened segment");
            active.encoder.write(&pcm[offset..offset + count])?;
            active.frames += count as u64;
            offset += count;
        }
        Ok(())
    }
    fn open<Tz: TimeZone>(&mut self, at: DateTime<Tz>, hour: String) -> Result<()>
    where
        Tz::Offset: std::fmt::Display,
    {
        let segment = self.store.create_segment(at.clone())?;
        let mut encoder = Encoder::new(segment.file.try_clone()?, self.config)?;
        if let Err(error) = self.store.mark_current(&segment) {
            let mut errors = vec![error];
            if let Err(error) = encoder.finish() {
                errors.push(error);
            }
            return Err(Error::Multiple(errors));
        }
        let path = self.store.path().join(&segment.relative);
        self.active = Some(Active {
            segment,
            encoder,
            hour,
            frames: 0,
        });
        if let Some(handler) = self.current.as_mut() {
            handler(Some(&path));
        }
        self.events.push(Event::Opened {
            path,
            acquired_at: at.fixed_offset(),
        });
        Ok(())
    }
    pub fn sync(&self) -> Result<()> {
        if let Some(active) = self.active.as_ref() {
            active.segment.file.sync_all()?;
        }
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        if let Some(handler) = self.current.as_mut() {
            handler(None);
        }
        let mut errors = Vec::new();
        if let Err(error) = self.store.clear_current() {
            errors.push(error);
        }
        if let Err(error) = active.encoder.finish() {
            errors.push(error);
        }
        if let Err(error) = active.segment.file.sync_all() {
            errors.push(error.into());
        }
        self.events.push(Event::Closed {
            path: self.store.path().join(&active.segment.relative),
            frames: active.frames,
            error: (!errors.is_empty()).then(|| crate::display_errors(&errors)),
        });
        if errors.is_empty() {
            Ok(())
        } else {
            Err(Error::Multiple(errors))
        }
    }
}
impl Drop for Sink {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
