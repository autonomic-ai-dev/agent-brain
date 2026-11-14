# Agent instructions for this repository

## Pull requests

- Split work into small PRs. One PR is one concern. Aim for under 400 changed lines. Split larger work before opening the PR.
- Stack PRs linearly on `master`. The first PR is based on `master`. Each next PR is based on the previous PR's branch. Do not base a PR on a branch from another stack.
- Keep the history linear. Rebase; do not merge `master` into a feature branch, and do not create merge commits.
- Merge the stack from the bottom. After a PR merges, rebase the next PR onto `master` and retarget its base.
- Rewrite a branch only with `git push --force-with-lease`, and only for your own unmerged branches.
- Branch names: `type/short-topic`, for example `refactor/r11-hook-rows`.
- Commit messages: Conventional Commits, `type(scope): summary`, with a short body that gives the reason.
- Write the PR description from `.github/pull_request_template.md`. Fill in the stack links: the base PR and the next PR.

## Before opening or merging a PR

- Run `.verify/run.sh` and make sure it exits 0. Record the result in the PR.
- If code changed, update the owning feature map in `.verify/features/` and its knowledge-base entry in `.verify/knowledge-base/` in the same PR.
- Do not add code comments, docstrings or TODOs unless the user asks for them.
- Do not use `--no-verify`, change `core.hooksPath`, or push to `master` directly.
