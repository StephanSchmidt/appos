.PHONY: check fmt build

## Before every commit: formatting, lints, unit tests.
check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

## Fixes the formatting that check complains about.
fmt:
	cargo fmt

## The kit, next to kit.toml: auth_github.wasm.
build:
	cargo build --release --target wasm32-wasip2
	cp target/wasm32-wasip2/release/auth_github.wasm .
