.PHONY: release test

release:
	cargo build --release -p tekmor

test:
	cargo test -p tekmor-core
	cargo test -p tekmor -- --test-threads=1
