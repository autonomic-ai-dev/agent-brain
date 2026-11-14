# Network egress statement

This page lists every code path in the tekmor binary and its bundled extension that can open a network connection. `tests/egress_guard.rs` checks the list against the source. If a source file starts or stops using a network client, the test fails until this page and the test's list are updated together.

## Summary

- No telemetry. The crate and the extension have no telemetry SDK dependency. The test checks the manifests.
- No model or LLM calls. No source file names a model provider host. The test checks that too.
- Routing, memory, verify, the bash and stop gates, the dashboard and `stats` work from local files and a local SQLite database. None of them open a network connection.
- Every network path is either something you ask for or a setting you turn on. Each one is listed below with its default.

## Network paths

| # | Feature | Source file | Trigger | Default | Destination |
|---|---|---|---|---|---|
| 1 | Documentation ingest (`learn_from_url`) | `src/docs/fetch.rs` | An agent or user asks to learn from a URL | Available, limited to `docs.allowed_domains` | HTTPS GET to an allowlisted host. Default list in `src/docs/allowlist.rs`. |
| 2 | Extension install (`tekmor ext add`) | `src/extensions.rs` | You run `ext add` | Only when you run it | The download URL in the extension manifest |
| 3 | Bench extension | `extensions/bench/src/skills_sh.rs` | You install the bench extension and run its skill commands | Only when installed and used | `https://skills.sh` search and download endpoints |
| 4 | Health endpoint | `src/mcp/server.rs` | `serve --daemon --health-addr <ADDR>` | Off. No address is set by default. | A TCP listener on the address you pass |
| 5 | Auto-update | `src/auto_update.rs` | Background loop in `serve --daemon` | Off (`auto_update.enabled: false`) | `curl` to `api.github.com` and `github.com` for release metadata and binary downloads. It can update the tekmor binary and installed packages. |
| 6 | Package install (`package add <url>`) | `src/packages/mod.rs` | You name a repository | Only when you run it | `git clone` and `git fetch` of the repository you name |
| 7 | Registry sync (`tekmor sync`) | `src/registry_sync.rs` | You run `sync` without `--local` | Only when you run it | `git fetch` and `git clone` of the configured registry repository on GitHub. `sync --local` uses the copy embedded in the binary. |
| 8 | Git sync | `src/sync/git.rs` | You configure a `sync.git.remote` and run its commands. Also `bootstrap_device` when a remote is set. | Off. The remote is empty by default. | `git push`, `pull`, `fetch`, and `clone` against your remote. `auto_push` pushes after writes and is off by default. |
| 9 | Cloud sync | `src/sync/cloud.rs` | `sync.cloud.enabled: true` with a bucket set | Off (`enabled: false`) | S3-compatible storage through `opendal`, or a local folder when the provider is `fs`. `auto_push` is also off by default. |

Item 4 is the only inbound listener. The daemon's main channel is a Unix socket named `serve.sock` in the tekmor home directory. That socket is not network.

## Not network

- `verify -- <cmd>` runs the allowlisted command on your machine. If that command uses the network (for example `npm test` fetching packages), the traffic belongs to that command. tekmor does not add it.
- `route_to_mcp` starts the MCP servers you configured as local child processes. Any network use belongs to those servers.
- The dashboard is a local HTML file with inline SVG and no script tags.
- Local git commands (`status`, `rev-parse`, `diff`) run on your files and do not contact a remote.

## How to check this yourself

1. Run `cargo test -p tekmor --test egress_guard`. It passes only when the list of network files matches the table above.
2. Run `tekmor serve` without `--daemon` and check with `lsof -i -P -n -p <pid>`. A default setup opens no network socket.
3. Run `tekmor dashboard`. It writes `metrics/dashboard.html` and opens no connection.

## Changing this page

If a change adds or removes a network client, update the table above and `EXPECTED_NETWORK_FILES` in `tests/egress_guard.rs` in the same commit. Check that any new outbound request is triggered by a user action or an opt-in setting, and that its destination is named here.
