---
feature: "[[../features/repo-tooling]]"
---

# Repo tooling

## What it does

Shell scripts, a Makefile and a Containerfile for installing, building, signing, releasing and migrating tekmor on a machine.

## Decisions

- The migration is a script that runs on each machine, not a runtime fallback. The binary does not read the old names. The script rewrites the host configs and shell files once, and keeps a backup of each file it changes.
- The migration copies `~/.agent_brain` to `~/.tekmor` and leaves the original in place, so a failed migration can be rolled back by restoring the backups.
- The shared memory store under `~/.autonomic/memory` is not renamed.
- The settings file is `~/.autonomic/memory/config.yaml`, which is the default memory directory. The migration rewrites that file. It does not write to `~/.tekmor/config.yaml`, because the binary never reads that path.
