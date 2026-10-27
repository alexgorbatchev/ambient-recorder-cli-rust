mod ambient_recorder 'crates/ambient-recorder/justfile'

default:
    @just --list

native-deps:
    just ambient_recorder native-deps

build:
    bash scripts/cargo.sh build --package ambient-recorder-cli --release --bin ambient-recorder
    mkdir -p bin
    cp target/release/ambient-recorder bin/ambient-recorder
    if test "$(uname -s)" = Darwin; then codesign --force --sign - bin/ambient-recorder && codesign --verify --strict bin/ambient-recorder; fi

[private]
_test:
    bash scripts/cargo.sh test --package ambient-recorder-cli --all-targets

[private]
_lint:
    bash scripts/cargo.sh clippy --package ambient-recorder-cli --all-targets -- -D warnings

[private]
_fmt:
    cargo fmt --package ambient-recorder-cli -- --check

# Report this CLI package separately from its internal crate.
coverage:
    mkdir -p .tmp
    bash scripts/cargo.sh llvm-cov --package ambient-recorder-cli --exclude-from-report ambient-recorder --all-targets --fail-under-lines 90 --json --output-path .tmp/coverage-final.json

check: _fmt _lint _test coverage
    just ambient_recorder check

run *args:
    bash scripts/cargo.sh run --package ambient-recorder-cli --bin ambient-recorder -- {{ args }}

run-ai *args:
    AGENT=1 bash scripts/cargo.sh run --package ambient-recorder-cli --bin ambient-recorder -- {{ args }}

test: _test
    just ambient_recorder test

lint: _lint
    just ambient_recorder lint

fmt: _fmt
    just ambient_recorder fmt-check
