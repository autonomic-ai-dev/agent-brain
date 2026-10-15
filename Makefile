.PHONY: release-macos test-release sync-release

release-macos:
	./scripts/build-release-macos.sh

test-release:
	cargo test --release -p agent-brain --features extras

sync-release:
	./scripts/sync-local-release.sh

.PHONY: container-check container-check-extras

# Apple Container (not Docker): build image and compile agent-brain
container-check:
	container build -t agent-brain-check -f Containerfile .

container-check-extras:
	container build -t agent-brain-check-extras -f Containerfile --build-arg FEATURES=extras .
