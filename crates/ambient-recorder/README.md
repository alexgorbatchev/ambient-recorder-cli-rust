`ambient-recorder` records local microphone and system playback as hourly mono Ogg Opus files, for applications that need private ambient audio and supervised recording.

# What It Does

- **Local audio**: Captures microphone and system playback on macOS 14.2 or later.
- **Hourly recordings**: Rotates files at local hour boundaries and distinguishes repeated daylight-saving hours.
- **Device recovery**: Applies microphone preferences and exclusions, retries failures, and preserves the current encoder during device switches.
- **Private supervision**: Maintains locked current-file identity, daily diagnostics, and a private Unix HTTP service endpoint.
- **Login service**: Generates and manages a per-user launchd agent with graceful restart.

# How It Works

1. Resolve configuration and validate microphone preferences and output settings.
2. Start recording with a cancellation token and optional current-file and microphone callbacks.
3. Read finalized Opus files and daily JSON diagnostics from the selected output directory.
4. Cancel to stop capture, drain pending audio, finalize the current file, and release recorder ownership.

# How it Really Works

- Recordings contain one 48 kHz mono stream. Defaults are 32,000 bit/s, complexity 2, and a five-second synchronization interval.
- Files live under `YYYY/MM/DD/` and carry nanosecond timestamps. Directories are created with mode 0700 and files with mode 0600. An active-recorder lock rejects concurrent writers.
- Acquisition timestamps drive file rotation. Input gaps and wall-clock corrections remain observable; the library does not fill downtime or promise gapless capture.
- A stalled input requires an empty capture read. Slow file or diagnostic writes do not classify a successfully producing microphone as stalled.
- Current-file queries verify native lock ownership and file identity and reject stale, foreign, and escaping paths.
- Audio, diagnostics, and service control remain local. The HTTP identity endpoint uses an owner-only Unix socket, with bounded reads and cancellation.
- Native capture owns its platform resources on the calling thread. Service control and identity calls require a Tokio runtime; importing the library starts no capture or service.
- A 90-second watchdog terminates the process with status 2 if the owner loop stops making progress. Capture errors can be aggregated with cleanup errors.

# Prerequisites

- macOS 14.2 or later with microphone and system-audio permissions for native capture. Other Unix platforms support the storage, codec, and controlled-audio APIs.
- Native [Opus](https://opus-codec.org/) and libopusenc libraries at build time.

# Installation

This unpublished implementation crate belongs to the `ambient-recorder-cli` repository. From a consumer beside that repository:

```sh
cargo add ambient-recorder --path ../ambient-recorder-cli/crates/ambient-recorder
```

# Quick Start

Encode supplied audio without microphone access:

```rust
use ambient_recorder::audio::{Encoder, EncoderConfig};

fn main() -> ambient_recorder::Result<()> {
    let mut encoder = Encoder::new(Vec::new(), EncoderConfig::default())?;
    encoder.write(&vec![0.0; 48_000])?;
    encoder.finish()?;
    assert!(encoder.writer().starts_with(b"OggS"));
    Ok(())
}
```

# API

| Entry point | Purpose |
| :--- | :--- |
| `config::Paths`, `config::Settings` | Resolve XDG paths, load strict TOML, apply overrides, validate and exclusively create configuration |
| `microphone::Preferences` | Order devices by case-sensitive name patterns, exact UID, exclusions and default input |
| `capture::Native` | Enumerate and capture native devices; returns `UnsupportedPlatform` outside macOS |
| `recording::run` | Own the capture/retry/storage lifecycle until cancellation or a fatal error |
| `recording::Sink`, `recording::Source` | Rotate acquired audio and resample supplied PCM |
| `storage::Store` | Create private files, acquire recorder ownership and inspect current-file identity |
| `service_info::Server`, `service_info::query` | Publish and read live process identity over a private Unix socket |
| `launchd::Agent` | Generate, install, inspect and manage an explicitly selected per-user service |

# Configuration

| Setting | Default | Meaning |
| :--- | :--- | :--- |
| `input.microphones` | `["*"]` | Ordered name/UID preferences; `!` excludes matching inputs globally |
| `output.dir` | `$XDG_DATA_HOME/ambient-recorder`, falling back to `~/.local/share/ambient-recorder` | Recording and diagnostic directory |
| `output.bitrate` | `32000` | Encoder bitrate, 6000–128000 |
| `output.complexity` | `2` | Encoder effort, 0–10 |
| `output.sync_interval` | `"5s"` | Positive compound duration with `ns`, `us`, `µs`, `μs`, `ms`, `s`, `m`, or `h` units |

Configuration defaults to `$XDG_CONFIG_HOME/ambient-recorder/config.toml`, falling back to `~/.config/ambient-recorder/config.toml`. An absent implicit file selects defaults; an absent explicitly selected file fails. Native macOS runtime capture, permissions, and launchd behavior require hardware verification; Linux fixture results and Darwin type checking do not prove them.

# License

[MIT](LICENSE), with native dependency notices in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
