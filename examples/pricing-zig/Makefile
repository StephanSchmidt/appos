.PHONY: check fmt dev

## Before every commit: formatting, the build and the unit tests.
check:
	zig fmt --check build.zig build.zig.zon src
	appos build
	zig build test

## Fixes the formatting that check complains about.
fmt:
	zig fmt build.zig build.zig.zon src

## Runs the app at http://127.0.0.1:8080/<app-id>; changes reload.
dev:
	appos dev
