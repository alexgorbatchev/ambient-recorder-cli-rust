use super::frame_duration;
use crate::{Result, audio::Resampler};
use chrono::{DateTime, Utc};

/// Conversion state belongs to one input session; the output sink can outlive
/// it across microphone changes. Every chunk re-anchors to its acquisition time.
pub struct Source {
    converter: Resampler,
    rate: u32,
    input: u64,
    output: u64,
    anchor: DateTime<Utc>,
}
impl Source {
    pub fn new(rate: f64) -> Result<Self> {
        Ok(Self {
            converter: Resampler::new(rate)?,
            rate: rate as u32,
            input: 0,
            output: 0,
            anchor: Utc::now(),
        })
    }
    pub fn write(
        &mut self,
        at: DateTime<Utc>,
        pcm: &[f32],
        persist: impl FnMut(DateTime<Utc>, &[f32]) -> Result<()>,
    ) -> Result<()> {
        let converted = self.converter.push(pcm)?;
        self.anchor = at - frame_duration(self.input, self.rate)?;
        self.input += pcm.len() as u64;
        self.persist(&converted, persist)
    }
    fn persist(
        &mut self,
        pcm: &[f32],
        mut persist: impl FnMut(DateTime<Utc>, &[f32]) -> Result<()>,
    ) -> Result<()> {
        if !pcm.is_empty() {
            persist(self.anchor + frame_duration(self.output, 48_000)?, pcm)?;
            self.output += pcm.len() as u64;
        }
        Ok(())
    }
    pub fn finish(
        &mut self,
        persist: impl FnMut(DateTime<Utc>, &[f32]) -> Result<()>,
    ) -> Result<()> {
        let tail = self.converter.finish()?;
        self.persist(&tail, persist)
    }
}
