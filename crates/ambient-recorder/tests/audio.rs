use ambient_recorder::{
    Error,
    audio::{Encoder, EncoderConfig, Resampler},
};
use std::{cell::Cell, f32::consts::TAU, io, process::Command};

#[test]
fn streaming_fft_conversion_preserves_duration_and_waveform_across_rates() {
    for rate in [16000, 44100, 48000, 96000] {
        let input: Vec<_> = (0..rate)
            .map(|i| 0.3 * (TAU * 880. * i as f32 / rate as f32).sin())
            .collect();
        for block in [137, 8192] {
            let mut resampler = Resampler::new(rate as f64).unwrap();
            assert_eq!(resampler.sample_rate(), rate as f64);
            let mut output = Vec::new();
            for chunk in input.chunks(block) {
                output.extend(resampler.push(chunk).unwrap());
            }
            output.extend(resampler.finish().unwrap());
            assert_eq!(output.len(), 48000, "{rate}/{block}");
            let error = (output
                .iter()
                .enumerate()
                .skip(512)
                .take(46976)
                .map(|(i, v)| (v - 0.3 * (TAU * 880. * i as f32 / 48000.).sin()).powi(2))
                .sum::<f32>()
                / 46976.)
                .sqrt();
            assert!(error < 0.01, "{rate}/{block}: RMS {error}");
            assert!(resampler.push(&[0.]).is_err());
            assert!(resampler.finish().unwrap().is_empty());
        }
    }
    for rate in [0., -1., f64::NAN, f64::INFINITY, 192001.] {
        assert!(Resampler::new(rate).is_err());
    }
}

#[derive(Default)]
struct FailingOutput {
    bytes: Vec<u8>,
    fail_write: Cell<bool>,
    fail_flush: Cell<bool>,
    writes: usize,
    flushes: usize,
}

impl io::Write for FailingOutput {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.fail_write.get() {
            return Err(io::Error::new(io::ErrorKind::BrokenPipe, "output closed"));
        }
        // Force write_all to preserve every byte across partial writes.
        let count = bytes.len().min(7);
        self.bytes.extend_from_slice(&bytes[..count]);
        Ok(count)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes += 1;
        if self.fail_flush.get() {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "flush failed"))
        } else {
            Ok(())
        }
    }
}

#[test]
fn encoder_propagates_header_and_finish_errors_and_cannot_resume_failed_streams() {
    let output = FailingOutput {
        fail_write: Cell::new(true),
        ..Default::default()
    };
    assert!(matches!(
        Encoder::new(output, EncoderConfig::default()),
        Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
    ));

    for fail_flush in [false, true] {
        let mut encoder = Encoder::new(FailingOutput::default(), EncoderConfig::default()).unwrap();
        assert!(encoder.writer().bytes.starts_with(b"OggS"));
        assert!(encoder.writer().writes > 1);
        encoder.write(&[]).unwrap();
        encoder.write(&[0.25; 480]).unwrap();
        encoder.writer().fail_flush.set(fail_flush);
        encoder.writer().fail_write.set(!fail_flush);
        assert!(matches!(
            encoder.finish(),
            Err(Error::Io(error)) if error.kind() == io::ErrorKind::BrokenPipe
        ));
        assert_eq!(encoder.writer().flushes, usize::from(fail_flush));
        let calls = (encoder.writer().writes, encoder.writer().flushes);
        assert!(matches!(encoder.write(&[0.5]), Err(Error::Invalid(_))));
        encoder.finish().unwrap();
        assert_eq!((encoder.writer().writes, encoder.writer().flushes), calls);
    }
}

#[test]
fn native_opus_pages_decode_and_trim_exactly_and_close_is_idempotent() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("speech.opus");
    let file = std::fs::File::create(&path).unwrap();
    let mut encoder = Encoder::new(file, EncoderConfig::default()).unwrap();
    assert!(
        std::fs::metadata(&path).unwrap().len() > 0,
        "headers published immediately"
    );
    let samples: Vec<_> = (0..48000)
        .map(|i| 0.3 * (TAU * 880. * i as f32 / 48000.).sin())
        .collect();
    for chunk in samples.chunks(137) {
        encoder.write(chunk).unwrap();
    }
    encoder.finish().unwrap();
    encoder.finish().unwrap();
    assert!(encoder.write(&[0.]).is_err());
    let output = Command::new("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(&path)
        .args([
            "-f",
            "f32le",
            "-acodec",
            "pcm_f32le",
            "-ar",
            "48000",
            "-ac",
            "1",
            "-",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout.len(), 48000 * 4);
    let rms = (output
        .stdout
        .as_chunks::<4>()
        .0
        .iter()
        .map(|b| f32::from_le_bytes(*b).powi(2))
        .sum::<f32>()
        / 48000.)
        .sqrt();
    assert!((0.1..0.5).contains(&rms));
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes.windows(8).filter(|w| *w == b"OpusHead").count(), 1);
    for config in [
        EncoderConfig {
            bitrate: 5999,
            ..Default::default()
        },
        EncoderConfig {
            complexity: 11,
            ..Default::default()
        },
        EncoderConfig {
            sample_rate: 0,
            ..Default::default()
        },
    ] {
        assert!(Encoder::new(Vec::new(), config).is_err());
    }
}
