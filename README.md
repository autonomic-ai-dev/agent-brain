# tekmor

A proof eval for coding agents. tekmor runs the checks a project allowlists, records what passed for the code as it is now, and blocks an agent from ending a turn while changed code has no passing proof.

## What it does

- **verify**: `tekmor verify -- <cmd>` runs an allowlisted command from `.verify/verify.toml` and records the outcome, with the code hash and commit.
- **receipt**: `tekmor verify receipt` says whether the current commit and code passed. It exits 0 only when it passed.
- **summary**: `tekmor verify summary` shows which checks fail most.
- **gates**: `tekmor stop-gate` and `tekmor bash-gate` run as Claude Code hooks. The stop gate blocks a turn with no passing proof or no feature map. The bash gate blocks or warns on risky shell commands and refuses commits that skip git hooks.
- **feature maps**: `.verify/features/` maps code to the feature it belongs to and the checks that prove it.

## Install

```
cargo build --release -p tekmor
tekmor install            # registers the gates in .claude/settings.json
tekmor install --global   # registers them in ~/.claude/settings.json
```

## Use

```
tekmor verify init                  # create .verify/ with an allowlist
tekmor verify -- .verify/run.sh     # run the project's checks and record them
tekmor verify receipt               # show the verdict for the current code
```

## Layout

- `tekmor/` — the binary and its CLI.
- `tekmor/crates/tekmor-core/` — pure proof types and the gate rule dispatcher, with no storage, runtime or IO dependencies.
- `.verify/` — this repo's own feature maps, allowlist and checks.

## Extensions

`tekmor ext list|add|remove` manages optional extension binaries. None are bundled.

## Development

```
cargo test -p tekmor-core
cargo test -p tekmor -- --test-threads=1
.verify/checks/core_boundary.sh
```

Verify this repo with `tekmor verify -- .verify/run.sh`.
