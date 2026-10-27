---
created_on: 2026-10-14 19:05
last_modified: 2026-10-14 22:40
status: current
---

# ambient-recorder

This repository contains the Rust CLI and its internal `crates/ambient-recorder` package in one Cargo workspace for native capture, recording, private identity and service control.

## Commands

- Prepare native codecs: `just native-deps`.
- Build: `just build`, producing `bin/ambient-recorder`.
- Run: `just run <args>` or `just run-ai <args>`.
- Test: `just test`.
- Lint: `just lint`.
- Format check: `just fmt`.
- Complete gate: `just check`.

Development requires Rust 1.98+, Cargo, just, a C compiler, libclang, pkg-config and make. Coverage requires cargo-llvm-cov and LLVM tools. Native codec preparation downloads checksum-verified Opus/libopusenc into `crates/ambient-recorder/.tmp/native`. Tests supply controlled PCM and local service subprocesses; they require no microphone access. Rust artifacts are unpublished.

## Contracts

- Consult applicable skills before changes. Scope formatting to this package; `cargo fmt --all` follows sibling path dependencies.
- Preserve the source application's offline embedded skill contract. `skills/ambient-recorder/SKILL.md` is the only canonical text; compile it with `include_str!` and print it verbatim in both modes.
- Update that skill and its metadata with every command, flag, default, environment, output or side-effect change. Keep the live-tree interface test and generated help alerts complete.
- Parse offline arguments and metadata before resolving paths or configuration. Explicit CLI overrides use Clap value provenance, so parser defaults never overwrite TOML.
- Keep recording on its calling thread; native capture objects are not moved to Tokio's generic blocking pool. Separate Tokio workers service the private identity endpoint.
- Publish absolute paths in service XML/live identity; fold home paths only when displaying results, logs and errors. Preserve invalid UTF-8 diagnostic bytes while folding valid text spans.
- Mac builds embed `Info.plist`, target macOS 14.2 and ad-hoc sign the final binary. A cross-typecheck does not verify linking, TCC permission identity, capture or launchd runtime behavior.
- Keep reusable audio/service implementation in `crates/ambient-recorder`. Its package coverage gate covers those sources; CLI coverage explicitly reports this package's sources.

## Boundaries

- Any code change affecting runtime results requires a corresponding test-file change and at least 90% line coverage; `scripts/` is excluded. Record red/green and genuine temporary-disable/restoration evidence under `.tmp`.
- Use controlled audio and local process/socket fixtures by default. Live microphone capture and changes to the user's login service require explicit authorization.
- Never publish releases, tags, packages or production deployments without explicit user authorization.
- Persist standing rules in place; keep progress, test outcomes and handoffs outside this file.
