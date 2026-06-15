.PHONY: release-macos test-release

release-macos:
	./scripts/build-release-macos.sh

test-release:
	cargo test --release -p agent-brain
