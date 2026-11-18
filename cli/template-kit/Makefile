.PHONY: check fmt build dev

## Before every commit: formatting, lints, unit tests.
check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

## Fixes the formatting that check complains about.
fmt:
	cargo fmt

## The kit's wasm, where kit.toml says. appos dev and appos deploy build it too.
build:
	cargo build --release --target wasm32-wasip2

## Runs the kit's page at http://127.0.0.1:8080/<kit-id>; changes reload.
dev:
	appos dev
