use crate::{Error, Result};
use opusenc::{
    Comments, MappingFamily,
    options::{Application, SignalType},
};
use rubato::{
    Fft, FixedSync, Indexing, Resampler as _, audioadapter_buffers::direct::InterleavedSlice,
};
use std::io::Write;

#[derive(Clone, Copy, Debug)]
pub struct EncoderConfig {
    pub sample_rate: i32,
    pub bitrate: i32,
    pub complexity: i32,
}
impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            sample_rate: 48000,
            bitrate: 32000,
            complexity: 2,
        }
    }
}
pub struct Encoder<W: Write> {
    native: Option<opusenc::Encoder>,
    writer: W,
}
fn codec(error: impl std::fmt::Display) -> Error {
    Error::Codec(error.to_string())
}
impl<W: Write> Encoder<W> {
    pub fn new(writer: W, config: EncoderConfig) -> Result<Self> {
        if !(1..=192000).contains(&config.sample_rate)
            || !(6000..=128000).contains(&config.bitrate)
            || !(0..=10).contains(&config.complexity)
        {
            return Err(Error::Invalid(
                "encoder rate must be 1..192000, bitrate 6000..128000 and complexity 0..10".into(),
            ));
        }
        let mut native = opusenc::Encoder::create_pull(
            Comments::create(),
            config.sample_rate,
            1,
            MappingFamily::MonoStereo,
        )
        .map_err(codec)?;
        native.set_application(Application::Voip).map_err(codec)?;
        native.set_signal_type(SignalType::Voice).map_err(codec)?;
        native.set_bitrate(config.bitrate).map_err(codec)?;
        native.set_complexity(config.complexity).map_err(codec)?;
        native.set_dtx(false).map_err(codec)?;
        native.set_decision_delay(0).map_err(codec)?;
        native.set_muxing_delay(4800).map_err(codec)?;
        native.flush_header().map_err(codec)?;
        let mut encoder = Self {
            native: Some(native),
            writer,
        };
        encoder.pages(false)?;
        Ok(encoder)
    }
    pub fn writer(&self) -> &W {
        &self.writer
    }
    fn pages(&mut self, flush: bool) -> Result<()> {
        if let Some(native) = self.native.as_mut() {
            while let Some(page) = native.get_page(flush) {
                self.writer.write_all(page)?;
            }
        }
        Ok(())
    }
    pub fn write(&mut self, samples: &[f32]) -> Result<()> {
        let native = self
            .native
            .as_mut()
            .ok_or_else(|| Error::Invalid("encoder is closed".into()))?;
        if !samples.is_empty() {
            native.write_float(samples).map_err(codec)?;
            self.pages(false)?;
        }
        Ok(())
    }
    pub fn finish(&mut self) -> Result<()> {
        if self.native.is_none() {
            return Ok(());
        }
        // Destroy even after a failed drain/write: this stream cannot safely resume.
        let result = (|| {
            self.native
                .as_mut()
                .expect("checked encoder")
                .drain()
                .map_err(codec)?;
            self.pages(true)?;
            self.writer.flush()?;
            Ok(())
        })();
        self.native.take();
        result
    }
}

/// Stateful mono FFT conversion. Delay is removed once; finish drains the tail.
pub struct Resampler {
    native: Option<Fft<f32>>,
    pending: Vec<f32>,
    rate: f64,
    input: u64,
    output: u64,
    delay: usize,
    closed: bool,
}
impl Resampler {
    pub fn new(rate: f64) -> Result<Self> {
        if !rate.is_finite() || !(1.0..=192000.0).contains(&rate) || rate.fract() != 0. {
            return Err(Error::Invalid(
                "sample rate must be an integer within 1..192000".into(),
            ));
        }
        let native = if rate == 48000. {
            None
        } else {
            Some(Fft::<f32>::new(rate as usize, 48000, 256, 1, FixedSync::Input).map_err(codec)?)
        };
        let delay = native.as_ref().map_or(0, |r| r.output_delay());
        Ok(Self {
            native,
            pending: Vec::with_capacity(512),
            rate,
            input: 0,
            output: 0,
            delay,
            closed: false,
        })
    }
    pub fn sample_rate(&self) -> f64 {
        self.rate
    }
    pub fn push(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        if self.closed {
            return Err(Error::Invalid("resampler is closed".into()));
        }
        self.input = self
            .input
            .checked_add(samples.len() as u64)
            .ok_or_else(|| Error::Invalid("audio frame count overflow".into()))?;
        if self.native.is_none() {
            self.output += samples.len() as u64;
            return Ok(samples.to_vec());
        }
        self.pending.extend_from_slice(samples);
        let mut output = Vec::new();
        while self.pending.len()
            >= self
                .native
                .as_ref()
                .expect("native converter")
                .input_frames_next()
        {
            let frames = self
                .native
                .as_ref()
                .expect("native converter")
                .input_frames_next();
            output.extend(self.process(frames)?);
        }
        Ok(output)
    }
    fn process(&mut self, valid: usize) -> Result<Vec<f32>> {
        let native = self.native.as_mut().expect("native converter");
        let frames = native.input_frames_next();
        let mut input = vec![0.; frames];
        input[..valid].copy_from_slice(&self.pending[..valid]);
        self.pending.drain(..valid);
        let adapter = InterleavedSlice::new(&input, 1, frames).map_err(codec)?;
        let mut output = native
            .process(&adapter, Some(&Indexing::new().partial_len(valid)))
            .map_err(codec)?
            .take_data();
        let skip = self.delay.min(output.len());
        self.delay -= skip;
        output.drain(..skip);
        // Padded tail may extend beyond the acquired input duration.
        let desired = (self.input as f64 * 48000. / self.rate).round() as u64;
        output.truncate(desired.saturating_sub(self.output) as usize);
        self.output += output.len() as u64;
        Ok(output)
    }
    pub fn finish(&mut self) -> Result<Vec<f32>> {
        if self.closed {
            return Ok(Vec::new());
        }
        self.closed = true;
        let mut output = Vec::new();
        if self.native.is_some() {
            if !self.pending.is_empty() {
                output.extend(self.process(self.pending.len())?);
            }
            let desired = (self.input as f64 * 48000. / self.rate).round() as u64;
            while self.output < desired {
                output.extend(self.process(0)?);
            }
        }
        Ok(output)
    }
}
