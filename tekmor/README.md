# tekmor (crate)

Rust MCP package for the tekmor binary. Product docs live in the [repository README](../README.md) and [docs/USAGE.md](../docs/USAGE.md).

```bash
cargo test --release -p tekmor
make -C .. release-macos   # Apple Silicon: build + Developer ID sign
cargo run --release -p tekmor -- serve
```

First run downloads the embedding model (~90MB) via fastembed.
