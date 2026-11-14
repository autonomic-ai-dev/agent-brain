# tekmor

Local MCP router for coding agents. It picks the skills, rules and memory a turn needs, enforces routing on every turn, and records real command exits as proof.

Named for *tekmērion* (τεκμήριον), Greek for proof or evidence.

## Install

```bash
cargo install tekmor
```

Then register it with your agent host:

```bash
tekmor install --global
tekmor doctor
```

`tekmor install --global` writes the MCP entry and the route-gate hooks for the hosts it finds. `tekmor doctor` checks the result.

## Use

- `tekmor serve` runs the MCP server over stdio. Hosts start it themselves.
- `tekmor verify -- <cmd>` runs a command and records its real exit code.
- `tekmor verify receipt` shows PASSED, FAILED or NOT VERIFIED for the current code.
- `tekmor package add <owner/repo>` installs a skill pack.

The first run downloads the embedding model (about 90 MB) through fastembed.

## Crates

| Crate | Purpose |
|-------|---------|
| `tekmor` | The binary: CLI, MCP server, hooks, verify recorder. |
| [`tekmor-core`](https://crates.io/crates/tekmor-core) | Routing types and scoring, with no storage or network IO. |

Optional feature `extras` adds the tree-sitter code index (Rust, TypeScript, Python, Go) and the eval and bench commands. Build it with `cargo install tekmor --features extras`.

## Docs

- [Repository README](https://github.com/autonomic-ai-dev/tekmor#readme)
- [Migration from agent-brain](https://github.com/autonomic-ai-dev/tekmor/blob/master/tekmor/docs/migrate-to-tekmor.md)
- [Changelog](https://github.com/autonomic-ai-dev/tekmor/blob/master/CHANGELOG.md)

## Build from source

```bash
cargo test --release -p tekmor
cargo run --release -p tekmor -- serve
```

## License

MIT
