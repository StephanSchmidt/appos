.PHONY: check fmt dev

## Before every commit: formatting, lints, unit tests.
check:
	cargo fmt --check
	cargo clippy --all-targets -- -D warnings
	cargo test

## Fixes the formatting that check complains about.
fmt:
	cargo fmt

## Runs the app at http://127.0.0.1:8080/<app-id>; changes reload.
dev:
	appos dev
