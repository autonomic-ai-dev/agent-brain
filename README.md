# tekmor

A local router for coding agents. tekmor gives the agent the skills, rules and memory a task needs, makes sure the agent routes every turn, and records what really passed.

[![crates.io](https://img.shields.io/crates/v/tekmor.svg)](https://crates.io/crates/tekmor)
[![Version](https://img.shields.io/badge/version-1.1.0--rc2-blue.svg)](CHANGELOG.md)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

tekmor runs on your machine over stdio MCP. Routing, memory and verification stay local.

## Why tekmor

- **Less context per turn.** The agent gets the few skills and rules that match the task, within a fixed token budget.
- **Routing that holds.** Host hooks block the agent's tool calls until the turn has been routed.
- **Proof, not claims.** `tekmor verify -- <cmd>` records a command's real exit code. A receipt says PASSED, FAILED or NOT VERIFIED for the code as it is now.
- **Shell guard.** Destructive shell commands are blocked before they run, and the block names a safe alternative.
- **Memory across sessions.** Facts you store are recalled in later sessions.

## Supported hosts

| Host | What tekmor sets up |
|------|---------------------|
| Claude Code | MCP server and route-gate hook |
| Cursor | MCP server, route-gate hook and rules |
| Codex | MCP server and hooks |
| Gemini CLI and Antigravity | MCP server and hooks |
| OpenCode | MCP server and route-gate plugin |
| Claude Desktop and VS Code | MCP server |

Hook enforcement depends on the host. Claude Desktop and VS Code get the MCP server only.

## Install

```bash
cargo install tekmor
tekmor install --global
tekmor doctor
```

`tekmor install --global` registers the MCP server and hooks with the hosts it finds. Add `--all` to write every host's files, or pick one host with a flag such as `--cursor`, `--claude-code` or `--vscode`. Restart the host afterwards.

`tekmor doctor` checks the binary, the hooks and the local store.

The first run downloads the embedding model (about 90 MB).

## Quick start

In a project, verify a change against a real command:

```bash
tekmor verify init
tekmor verify -- cargo test
tekmor verify receipt
```

`verify receipt` prints PASSED only when the recorded run passed on the current code. Add `--markdown` to paste the receipt into a pull request. The walk-through is in [docs/first-receipt.md](docs/first-receipt.md).

## Commands

| Command | What it does |
|---------|--------------|
| `tekmor serve` | Runs the MCP server. Hosts start it for you. |
| `tekmor install` | Registers tekmor with hosts. |
| `tekmor doctor` | Checks the install and the local store. |
| `tekmor verify -- <cmd>` | Runs a command and records its exit code. |
| `tekmor verify receipt` | Shows PASSED, FAILED or NOT VERIFIED for the current code. |
| `tekmor dashboard --days 7` | Writes a local HTML page with routing figures. |
| `tekmor stats` | Prints routing and token counters in the terminal. |
| `tekmor remember` | Stores a fact for later sessions. |
| `tekmor package add <owner/repo>` | Installs a skill pack. |
| `tekmor sync` | Syncs memory and skills between machines. |

Run `tekmor --help` for the full list.

## Configuration

tekmor reads its settings from `~/.autonomic/memory/config.yaml`. Set `TEKMOR_HOME` to use another directory. Environment variables use the `TEKMOR_` prefix.

The memory store is in `~/.autonomic/memory`. It is shared with other Autonomic AI tools.

## Coming from agent-brain

agent-brain is now tekmor. The binary, config and environment variables changed names. Your data stays where it was. Read the [migration guide](tekmor/docs/migrate-to-tekmor.md) and run `scripts/migrate-to-tekmor.sh --dry-run` before you apply the change.

## Documentation

- [Changelog](CHANGELOG.md)
- [Support matrix](docs/support-matrix.md): what each host gets
- [Network egress](docs/egress.md): every network call tekmor can make
- [First receipt](docs/first-receipt.md): your first verified change
- [Migration from agent-brain](tekmor/docs/migrate-to-tekmor.md)

## Name

tekmor is a short spelling of *tekmērion* (τεκμήριον), Greek for proof or evidence: the sure sign that settles a question. Sources: [Strong's G5039 on Bible Hub](https://biblehub.com/greek/5039.htm) and [OpenBible](https://openbible.com/strongs/greek/5039.htm).

## Related

- [thesmos](https://github.com/autonomic-ai-dev/thesmos) runs multi-step workflows with approval gates. It works alongside tekmor.

## Contributing

```bash
cargo test --release -p tekmor
.verify/run.sh
```

Pull requests run the same checks in CI.

## License

MIT. See [LICENSE](LICENSE).
