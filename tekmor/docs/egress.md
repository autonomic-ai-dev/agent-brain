# Network egress

tekmor makes no network connection on its own. It opens a network connection only in the two places below, and only when you ask for an extension.

| # | Path | Source | When it runs | Default | What is sent |
|---|------|--------|--------------|---------|--------------|
| 1 | Extension install | `src/extensions.rs` | You run `tekmor ext add <name>` | Only when you run it | A download of the extension binary and its manifest |
| 2 | Registry sync | `src/registry_sync.rs` | An extension install needs the registry manifest | Only when an install needs it | A git fetch of the registry repository |

`verify -- <cmd>` runs the allowlisted command on your machine. If that command uses the network (for example `npm test` fetching packages), that traffic belongs to the command, not to tekmor.

To check the statement yourself:

1. Read the table above and the two source files it names.
2. Run `lsof -i -P -n -p <pid>` against a running `tekmor stop-gate` or `tekmor bash-gate`. Neither opens a network socket.
