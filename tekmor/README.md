# tekmor

The tekmor crate: the `tekmor` binary (a proof eval for coding agents) and the `tekmor-core` crate it depends on.

Run `tekmor help` for the commands. The product overview is in the repository root README.

Build and test:

```
cargo build --release -p tekmor
cargo test -p tekmor -- --test-threads=1
cargo test -p tekmor-core
```
