# Allowlist and runner

Load this file when you set up or change `.verify/verify.toml` or `.verify/run.sh`.

## Discovery

Find the test setup by reading the repository, not by asking the user:

- Unit test frameworks: the build manifest or package file (for example `Cargo.toml`, `pyproject.toml`, `package.json`, `go.mod`, `Makefile`, or a Maven or Gradle build file).
- Endpoints for functional tests: the routing tables, controllers, or handlers, with their paths, request schemas, status codes, and verbs.
- Environment and ports: example environment files, configuration files, container manifests, and command-line flags.

## Layout

```text
.verify/
├── verify.toml        allowlist of trusted commands
├── run.sh             runner: doctor, unit, static, functional
├── doctor.sh          read-only environment checks
├── functional/        Hurl files, one per feature, with payloads/
└── artifacts/         run output, kept after teardown
```

## Allowlist (`verify.toml`)

The allowlist names every command that counts as verification evidence. A command that is not listed is run at most as an exploration, and its result is not evidence.

```toml
[verify]
commands = [
  ".verify/run.sh",
  ".verify/run.sh all",
  "hurl --test .verify/functional/account-creation.hurl",
]

[verify.default]
command = [".verify/run.sh"]
timeout_secs = 900
hook = true
```

Add a command to the allowlist before you rely on it as evidence. Write each entry as a literal command.

## Runner (`run.sh`)

The runner executes four phases in order:

1. **Doctor:** run `doctor.sh`. A failing doctor stops the run before any test starts.
2. **Unit:** run the repository's unit tests for the selected feature, or for the whole repository when no feature is given.
3. **Static:** run the static checks: coverage of feature entries, payload contracts, and the repository's cluster or configuration checks.
4. **Functional:** run the Hurl files against the local services. Pass credentials through environment variables, and never print them.

The runner:

- takes one argument, a feature name or `all`;
- writes every log and report under `.verify/artifacts/<RUN_ID>/`;
- exits non-zero when any phase fails;
- exits with a distinct code when a phase is blocked, for example by a missing credential, so the blocked state is never mistaken for a pass.

## Doctor (`doctor.sh`)

The doctor is read-only. It checks the required tools, the required ports, and that the required credentials are set. It reports the presence of a credential, never its value.
