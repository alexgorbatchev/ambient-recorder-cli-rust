---
created_on: 2026-10-14 18:31
last_modified: 2026-10-14 22:40
status: current
---

# ambient-recorder

Native capture, acquired-time recording, private storage, and per-user service primitives. This internal workspace package belongs to `ambient-recorder-cli`; keep its APIs independent of CLI parsing and formatting. Inherit `../../AGENTS.md` and run the following commands from this package directory.

## Commands

- Native codec setup: `just native-deps` builds checksum-verified static Opus/libopusenc under `.tmp/native`.
- Build: `just build`.
- Tests: `just test` (requires FFmpeg and ffprobe for real codec fixtures).
- Format: `cargo fmt --package ambient-recorder`.
- Strict checks: `just check`.

## Native contracts

- Use maintained objc2 Core Audio/AVFoundation bindings directly. Keep native capture on its owning thread and callbacks allocation-free; queue overflow drops new input and reports the lost frame count.
- Preserve acquired-time rotation, repeated DST hours, resampler delay removal and final tail draining. Microphone switches retain the current output encoder.
- Check input stalls only on empty reads; persistence latency must not mark producing inputs stalled.
- Keep recorder/current-file native locks, inode identity, capability-relative path confinement, and nonblocking inspection of foreign FIFO paths.
- Preserve typed cleanup errors and stop/drain/finalize/close ownership, including cancellation and partial initialization.
- Service restart waits for launchd job disappearance before starting another process. Keep full native stdout/stderr diagnostics and bounded descriptor draining.
- Saved service arguments are interpreted by the consumer's actual argument parser. Running identity takes precedence over saved configuration; job loading alone does not prove recording works.

## Verification and boundaries

- Runtime-result changes require corresponding test-file changes and at least 90% line coverage; `scripts/` is excluded. Use red/green and genuine behavior-disable/restoration proofs.
- Keep controlled audio and subprocess fixtures separate from opt-in native capture. Never capture user audio or install/start persistent services as a build or test side effect.
- Darwin cross-checks do not prove macOS linking, permission flows, capture quality, or launchd execution. Report native runtime limitations explicitly.
- Keep generated native dependencies, evidence, and temporary files under `.tmp/`. Preserve native license notices when distributing linked artifacts.
- Rust artifacts are unpublished. Never publish releases, tags, packages, or deployments without explicit authorization.
