.PHONY: all native template examples typescript zig java check check-dev check-console run clean

## Everything but typescript, zig and java, which need Node.js, zig and a JDK.
all: native template examples check check-dev check-console

## The shell (appos-shell) and the CLI (appos).
native:
	cargo build --release

## The hello world every `appos new` starts from must always build and pass its check.
template:
	cd cli/template && cargo build --release --target wasm32-wasip2
	cd cli/template && $(MAKE) check

## Every example app builds for wasm and passes its own check. Each carries
## the SDK and the contract of cli/template (cargo test -p appos checks).
examples:
	for app in $(dir $(wildcard examples/*/Cargo.toml)); do \
		(cd $$app && cargo build --release --target wasm32-wasip2 && $(MAKE) check) || exit 1; \
	done

## TypeScript apps: a new `appos new --ts` app passes its check, and the
## TypeScript examples build. Needs Node.js 24+ and zig, so not in `all`.
typescript: native
	rm -rf target/typescript && mkdir -p target/typescript
	cd target/typescript && PATH=$(CURDIR)/target/release:$$PATH appos new hello --ts && cd hello && PATH=$(CURDIR)/target/release:$$PATH $(MAKE) check
	for app in $(dir $(wildcard examples/*/package.json)); do \
		(cd $$app && $(CURDIR)/target/release/appos build) || exit 1; \
	done

## Zig apps: the Zig examples pass their check (build and unit tests).
## Needs zig 0.17, so not in `all`.
zig: native
	for app in $(dir $(wildcard examples/*/build.zig)); do \
		(cd $$app && PATH=$(CURDIR)/target/release:$$PATH $(MAKE) check) || exit 1; \
	done

## Java apps: a new `appos new --java` app passes its check (build and unit
## tests), as do the Java examples. Needs a JDK 21+ and Maven, so not in `all`.
java: native
	rm -rf target/java && mkdir -p target/java
	cd target/java && PATH=$(CURDIR)/target/release:$$PATH appos new hello --java && cd hello && PATH=$(CURDIR)/target/release:$$PATH $(MAKE) check
	for app in $(dir $(wildcard examples/*/pom.xml)); do \
		(cd $$app && PATH=$(CURDIR)/target/release:$$PATH $(MAKE) check) || exit 1; \
	done

## fmt, clippy (a warning fails) and the tests of the shell and the CLI.
check:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets -- -D warnings
	cargo test --workspace

## Live dev-server regressions; requires Python 3 and the wasm32-wasip2 target.
check-dev:
	cargo build -p appos
	python3 cli/tests/dev_reload.py

## Console controls, multiline app logs and closed log pipes, through both binaries.
check-console:
	cargo build --workspace
	python3 cli/tests/terminal_output.py

## The shell with the hello world.
run: all
	target/release/appos-shell --risk risk.toml cli/template

clean:
	rm -rf target cli/template/target appos-data
