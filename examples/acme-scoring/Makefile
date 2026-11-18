.PHONY: check fmt build

## Before every commit: formatting, lints, unit tests.
check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

## Fixes the formatting that check complains about.
fmt:
	cargo fmt

## The kit's wasm, where kit.toml says. `appos deploy` builds it too.
build:
	cargo build --release --target wasm32-wasip2
