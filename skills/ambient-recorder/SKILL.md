---
name: ambient-recorder
description: Use when operating ambient-recorder to record local audio, configure microphone preferences, inspect inputs, or manage its background service.
author: alexgorbatchev
metadata:
  created_on: 2026-10-14 18:33
  last_modified: 2026-10-14 19:05
  status: current
---

## Recording workflow

Use macOS 14.2 or later for microphone and computer-playback capture. Grant microphone and system-audio permissions to the executable. Audio and diagnostics remain local. Rust release artifacts are unpublished; native macOS capture and launchd runtime verification remain pending.

```sh
AGENT=1 ambient-recorder microphone list
AGENT=1 ambient-recorder config init
AGENT=1 ambient-recorder recording start
```

Edit the created TOML before recording. Stop with Ctrl-C or SIGTERM to drain captured audio and finalize the active file. Configuration is read once; restart to apply edits. Foreground recording requests capture permissions; service installation immediately requests background recording.

## Commands

All domain leaves and `skill` accept no positional arguments. Groups print their subtree. Results use stdout; recording diagnostics and returned errors use stderr. Path displays fold the home directory to `~` in both modes. Filesystem calls, saved plist values and live identity retain absolute paths.

| Command | Contract |
| :--- | :--- |
| `ambient-recorder` | Print root help. |
| `ambient-recorder config` | Print configuration help. |
| `ambient-recorder config init` | Exclusively create default TOML and parent directories; refuse an existing file. Print `Configuration created: <path>` or agent `config=<path>`. |
| `ambient-recorder config print-file` | Print only the selected existing configuration path and newline, without parsing TOML. Symlinks to regular files are accepted. Missing/nonregular files leave stdout empty and exit 1. |
| `ambient-recorder microphone` | Print input help. |
| `ambient-recorder microphone list` | Enumerate input names, stable UIDs, transient IDs, default status, transport, type, manufacturer and model without opening capture. Empty inventory prints `No microphones available`. |
| `ambient-recorder recording` | Print recording help. |
| `ambient-recorder recording print-dir` | Load settings and print only the resolved recording directory and newline; create nothing. |
| `ambient-recorder recording print-file` | Print only the verified open segment and newline. No active segment leaves stdout empty and exits 1. This is a snapshot of ownership, not capture health. |
| `ambient-recorder recording start` | Capture microphone plus computer playback as mono Ogg Opus until cancelled. Create private files/directories, daily diagnostics and an exclusive recorder lock. |
| `ambient-recorder service` | Print service help. |
| `ambient-recorder service print` | Print launchd XML with resolved absolute executable/output/configuration paths; install and start nothing. XML is byte-identical between modes and keeps operational paths absolute. |
| `ambient-recorder service install` | Create private LaunchAgent plist, create default TOML if absent, enable GUI-login startup and request recording immediately. Refuse an existing plist; retain a saved plist on launch failure. |
| `ambient-recorder service start` | Refresh saved executable/socket settings, preserve other options, enable startup and bootstrap or kickstart. Require installation; an unchanged loaded job is not interrupted. |
| `ambient-recorder service stop` | Disable startup and unload, waiting for job disappearance. Retain plist, TOML and recordings. |
| `ambient-recorder service restart` | Refresh, unload and wait for disappearance, then enable and bootstrap the saved job. |
| `ambient-recorder service uninstall` | Disable startup, unload and remove plist; retain TOML/audio/diagnostics. An absent plist is accepted. |
| `ambient-recorder service status` | Show installed/loaded states, invoked CLI and live service versions, plist, output/configuration/current paths, active microphone and path provenance. Optional native diagnostics follow. |
| `ambient-recorder skill` | Print this embedded Markdown verbatim, including frontmatter, offline. |
| `ambient-recorder help [COMMAND]...` | Print help for a space-separated command path. |
| `ambient-recorder completion` | Print completion help. |
| `ambient-recorder completion bash` | Print a Bash completion script. |
| `ambient-recorder completion fish` | Print a Fish completion script. |
| `ambient-recorder completion powershell` | Print a PowerShell completion script. |
| `ambient-recorder completion zsh` | Print a Zsh completion script. |

Generated `help` queries accept a command path; completion generation installs nothing. Every command supports `--help`.

These generated queries display the named command's help and perform no domain operation:

| Generated help query | Target |
| :--- | :--- |
| `ambient-recorder help config` | Configuration group |
| `ambient-recorder help config init` | Configuration creation |
| `ambient-recorder help config print-file` | Configuration path |
| `ambient-recorder help microphone` | Input group |
| `ambient-recorder help microphone list` | Input inventory |
| `ambient-recorder help recording` | Recording group |
| `ambient-recorder help recording start` | Foreground recording |
| `ambient-recorder help recording print-dir` | Output directory |
| `ambient-recorder help recording print-file` | Current file |
| `ambient-recorder help service` | Service group |
| `ambient-recorder help service print` | Service XML |
| `ambient-recorder help service install` | Installation |
| `ambient-recorder help service start` | Start |
| `ambient-recorder help service stop` | Stop |
| `ambient-recorder help service restart` | Restart |
| `ambient-recorder help service uninstall` | Removal |
| `ambient-recorder help service status` | Status |
| `ambient-recorder help skill` | Usage guide |
| `ambient-recorder help completion` | Completion group |
| `ambient-recorder help completion bash` | Bash |
| `ambient-recorder help completion fish` | Fish |
| `ambient-recorder help completion powershell` | PowerShell |
| `ambient-recorder help completion zsh` | Zsh |
| `ambient-recorder help help` | Help queries |
| `ambient-recorder config help` | Configuration group |
| `ambient-recorder config help init` | Configuration creation |
| `ambient-recorder config help print-file` | Configuration path |
| `ambient-recorder config help help` | Help queries |
| `ambient-recorder microphone help` | Input group |
| `ambient-recorder microphone help list` | Input inventory |
| `ambient-recorder microphone help help` | Help queries |
| `ambient-recorder recording help` | Recording group |
| `ambient-recorder recording help start` | Foreground recording |
| `ambient-recorder recording help print-dir` | Output directory |
| `ambient-recorder recording help print-file` | Current file |
| `ambient-recorder recording help help` | Help queries |
| `ambient-recorder service help` | Service group |
| `ambient-recorder service help print` | Service XML |
| `ambient-recorder service help install` | Installation |
| `ambient-recorder service help start` | Start |
| `ambient-recorder service help stop` | Stop |
| `ambient-recorder service help restart` | Restart |
| `ambient-recorder service help uninstall` | Removal |
| `ambient-recorder service help status` | Status |
| `ambient-recorder service help help` | Help queries |
| `ambient-recorder completion help` | Completion group |
| `ambient-recorder completion help bash` | Bash |
| `ambient-recorder completion help fish` | Fish |
| `ambient-recorder completion help powershell` | PowerShell |
| `ambient-recorder completion help zsh` | Zsh |
| `ambient-recorder completion help help` | Help queries |

## Flags

Explicit flags override TOML, which overrides built-in defaults. `path` values may be relative to the process working directory. Empty parser defaults select settings; an explicitly empty `--output` fails before execution.

| Scope | Flag and type | Default and meaning |
| :--- | :--- | :--- |
| All commands | `--config` path | Empty `""`: use XDG default; an explicitly named missing file fails where settings are loaded. Ignored by microphone, skill, help, completion and saved-service operations. |
| All commands | `--help`, `-h` bool | `false`: print help and exit 0. |
| Root | `--version`, `-v` bool | `false`: print raw `0.1.0` plus newline. |
| `recording start`, `recording print-file`, `service print`, `service install` | `--output` path | Empty `""`: inherit configured/default output. Explicitly empty fails. |
| `recording start` | `--bitrate` int32 | `32000`: 6000..128000 bits/s. |
| `recording start` | `--complexity` int32 | `2`: effort 0..10. |
| `recording start` | `--sync-interval` duration | `5s`: positive duration such as `500ms`, `5s`, or `1m30s`; units `ns`, `us`, `µs`, `μs`, `ms`, `s`, `m`, `h`. |
| `service status` | `--details` bool | `false`: append native launchctl diagnostics, with display-only home folding. |
| Completion shell leaves | `--no-descriptions` bool | `false`: omit descriptions from choices. |

Boolean command flags accept `=true` or `=false`. Unexpected arguments, unknown flags and invalid values fail with one returned error on stderr.

## Configuration and device preferences

```toml
[input]
microphones = ["USB*", "!Virtual*", "*"]
[output]
dir = "recordings"
bitrate = 32000
complexity = 2
sync_interval = "5s"
```

Use strict `[input]`/`[output]` tables or dotted root keys. Unknown keys/types fail. Omitted defaults are microphones `["*"]`, bitrate 32000, complexity 2 and interval 5s. Relative TOML output resolves against working directory; `~` and environment-variable text remain literal.

Order preferred names/patterns first. Only `*` expands, case-sensitively across the whole name. `uid:<UID>` matches an exact stable UID. `!` globally excludes a name/pattern/UID; exclusions override positives and fallback. Empty selectors, bare `!` and empty UIDs fail. An implicit final `*` retains eligible unlisted inputs; `[]` allows all. Wildcard fallback selects the system default first, then stable UID/ID order.

## Environment and streams

- `AGENT`: trimmed case-insensitive `1`, `true`, `yes` selects compact output; other values select human mode. Agent help starts with the skill-reading alert. Human help fits `COLUMNS`/terminal width with an 80-column fallback.
- `HOME`: home directory for defaults; `XDG_CONFIG_HOME` and `XDG_DATA_HOME` are absolute overrides. Configuration is `ambient-recorder/config.toml` beneath the config base (`~/.config` fallback). Output is `ambient-recorder` beneath data base (`~/.local/share` fallback). Missing implicit TOML selects defaults; explicitly selected missing TOML fails.
- `NO_COLOR`: any nonempty value disables recording log colors. `TERM=dumb` and redirected stderr also disable colors.
- `AMBIENT_RECORDER_SERVICE_SOCKET`: generated service's private Unix socket. Recording publishes version, absolute startup paths, current segment and microphone until shutdown. Empty/unset leaves foreground recording without identity endpoint. Startup failure exits before capture; socket queries time out after one second.
- Human microphone output includes quoted names/default status, transport/type and optional metadata/UID preference. Agent output is one `name=<quoted> uid=<quoted> device_id=<integer> default=<bool> transport=<quoted> type=<quoted> manufacturer=<quoted> model=<quoted>` line per device.
- Service actions print a result and `Service file: <path>`; agent mode prints `service=<action> plist=<quoted path>`. Human status is a borderless two-column table. Agent status is one line: `installed=<bool> loaded=<bool> cli_version=<quoted> service_version=<quoted> plist=<quoted> output_dir=<quoted> config_path=<quoted> current_file=<quoted> paths_source=<quoted> microphone=<quoted>`.
- Status paths come from live process (`live`), saved arguments (`saved`) or `unavailable`. Saved arguments use the actual command parser without loading current TOML. Saved paths must be absolute. Unloaded jobs ignore sockets and report service version/current/microphone `not running`. Loaded jobs missing identity report `unavailable`. Empty reported microphone/current displays `none`; empty live config displays `none (defaults)`. Job loading and identity do not prove arriving audio.
- Recording writes INFO/WARN/ERROR to stderr, compact JSON in agent mode; daily JSON includes DEBUG and inventory. Files are `YYYY/MM/DD/log.nljson`. Failed daily logging reports `log_error`; console failures are best effort.
- Normal completion/cancellation exits 0. Invalid arguments/configuration, startup and fatal cleanup failures exit 1. Retryable capture/storage failures log and retry. A 90-second watchdog exits 2 for supervisor recovery. Main errors begin `[ERROR]` in human mode or `ERR:` in agent mode.

## Files and services

Recordings are mono 48 kHz Ogg Opus under `YYYY/MM/DD/HH-MM-SS.000000000.opus`, based on acquisition time, rotating at local hour boundaries including repeated daylight-saving hours. Files are reserved exclusively, directories 0700, files/logs 0600, retention unlimited. Device switches preserve the active file/encoder and can leave gaps. Five seconds without input defers that microphone for 30 seconds; slow successful persistence does not count as an input stall.

The recorder holds `.recording.lock`, publishes relative `.recording.current` after encoder setup, and clears it before finalization. Current-file queries require native live ownership and reject stale/foreign/escaping targets. Pages appear approximately every 100 ms and synchronization defaults to five seconds; crashes can lose buffered audio or leave a stream without final trimming. Storage recovery may duplicate pending samples. Diagnostics report gaps/dropped samples; successful file finalization does not prove uninterrupted capture.

Grant capture permissions in foreground before installing the service. Keep the executable at its permanent path. Run service commands as the logged-in user without sudo. Installation saves `~/Library/LaunchAgents/com.alexgorbatchev.ambient-recorder.plist`, pins absolute `--output` and `--config`, and creates absent default TOML. Start/restart refresh the invoked executable and live socket while preserving unknown plist fields, environment and recording options. Reinstall to change pinned paths; restart to reload TOML.

The Aqua-session job runs at GUI login, uses KeepAlive and a five-second throttle, and cannot capture during sleep/before login. Stop/restart wait up to two minutes for job disappearance. Cancellation/shutdown errors fail the command. Inspect daily JSON and `supervisor.stderr.log`; system permissions/Background Items settings can prevent capture. The `.ar-<eight hexadecimal digits>.sock` endpoint uses 0600 permissions and a retained `.sock.lock`. Native socket path limits apply.
