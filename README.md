`ambient-recorder` records microphone and computer playback locally as hourly Ogg Opus files, for people who need private audio capture and recording at login on macOS.

# What It Does

- **Local recording**: Captures microphone and computer playback in one mono stream.
- **Input recovery**: Orders preferred microphones and exclusions, retries unavailable inputs, and preserves the active file across device changes.
- **Hourly files**: Rotates at acquisition-time hour boundaries and distinguishes repeated daylight-saving hours.
- **Private supervision**: Reports the active microphone and file, with daily JSON diagnostics and a per-user login service.
- **Offline guidance**: Prints its complete embedded usage guide with `ambient-recorder skill`.

# How It Works

- List microphones and create a default TOML configuration.
- Edit preferences and choose an output directory, then start foreground recording.
- Grant microphone and system-audio access to the executable when prompted.
- Stop with Ctrl-C or SIGTERM to drain captured audio and finalize the active file.
- Install the login service after checking permissions in foreground; installation requests recording immediately.

# How it Really Works

- Defaults are 48 kHz mono, 32,000 bit/s, complexity 2 and a five-second storage synchronization interval. Files and logs live under `~/.local/share/ambient-recorder/YYYY/MM/DD/`, unless configuration or flags select another directory.
- Configuration defaults to `~/.config/ambient-recorder/config.toml`, with XDG overrides. Explicit flags override TOML; missing implicit TOML selects defaults. Relative output paths resolve against the working directory.
- Recordings and diagnostics remain local. Directories are created with mode 0700 and files with mode 0600. Retention is unlimited; input gaps and dropped samples remain observable in diagnostics.
- A live-ownership check distinguishes the active file from a stale pointer. Service status reports live process paths when available, otherwise saved absolute arguments, without reloading current TOML.
- Normal results go to stdout; recording diagnostics and returned errors go to stderr. `AGENT=1` selects compact output. Displayed home paths use `~`, while generated service XML retains operational absolute paths.
- Cancellation exits 0 after cleanup. Invalid arguments, configuration, startup or fatal cleanup errors exit 1. A stalled owner-loop watchdog exits 2 so a supervisor can restart it.
- Slow successful persistence does not classify a producing microphone as stalled. Storage recovery can duplicate pending samples; file finalization does not prove uninterrupted capture.

# Prerequisites

- [macOS](https://www.apple.com/macos/) 14.2 or later for native microphone and system playback capture, with permission for the executable. Linux supports offline commands and controlled-audio tests; native capture returns an unsupported-platform error.
- A logged-in graphical macOS session for the per-user launchd service. Native macOS capture, permission identity and launchd runtime verification remain pending.

# Installation

Rust release artifacts are unpublished. Existing Go release archives do not contain this Rust executable. Local development build instructions are in [AGENTS.md](AGENTS.md).

# Quick Start

```sh
ambient-recorder microphone list
ambient-recorder config init
ambient-recorder recording print-dir
```

Sample output for the default recording-directory query:

```text
~/.local/share/ambient-recorder
```

After editing the configuration and granting capture permissions:

```sh
ambient-recorder recording start
ambient-recorder service install
ambient-recorder service status --details
```

# Options & Flags

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--config <path>` | | Empty `""` | Select TOML; empty selects the XDG configuration |
| `--help` | `-h` | `false` | Print command help |
| `--version` | `-v` | `false` | Print only the raw version |

`ambient-recorder recording start`

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--output <path>` | | Empty `""` | Inherit configured output; explicitly empty fails |
| `--bitrate <integer>` | | `32000` | Opus bitrate, 6000–128000 bits/s |
| `--complexity <integer>` | | `2` | Encoder effort, 0–10 |
| `--sync-interval <duration>` | | `5s` | Positive compound duration, such as `500ms` or `1m30s` |

`ambient-recorder recording print-file`, `ambient-recorder service print`, `ambient-recorder service install`

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--output <path>` | | Empty `""` | Inherit configured output; explicitly empty fails |

`ambient-recorder service status`

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--details` | | `false` | Append native service diagnostics |

`ambient-recorder completion bash`, `fish`, `powershell`, `zsh`

| Flag | Short | Default | Description |
| :--- | :--- | :--- | :--- |
| `--no-descriptions` | | `false` | Omit completion descriptions |

# Configuration and Service

Use `ambient-recorder skill` for complete selectors, configuration, environment variables, output fields and service contracts. `service print` previews XML without installing. Service start/restart refresh the executable and identity socket while preserving saved recording options and other plist fields; uninstall retains recordings and configuration.

# License

[MIT](LICENSE), with codec attribution in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
