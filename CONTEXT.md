# CONTEXT

tekmor is a proof eval. Terms used in this repo:

- **Check**: a command in `.verify/verify.toml`.
- **Run**: one recorded execution of a check, with its exit code, the commit and a hash of the code.
- **Receipt**: the verdict for the current commit and code, from the recorded runs.
- **Feature map**: a file in `.verify/features/` that owns code and names the checks that prove it.
- **Gate**: a hook that blocks an agent action: the stop gate for turn end, the bash gate for shell commands.
