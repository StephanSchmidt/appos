.PHONY: check dev

## Before every commit: the build (javac, TeaVM, the component), unit tests.
check:
	appos build
	mvn -q -B test

## Runs the app at http://127.0.0.1:8080/<app-id>; changes reload.
dev:
	appos dev
