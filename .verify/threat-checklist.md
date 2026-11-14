# Agent threat checklist (Stage 1 / review finding #3)

Before running agent workflows against a repo:

- [ ] No production credentials in the worktree, Compose files, or `.env` committed to git
- [ ] Tokens/secrets are scoped and short-lived; revoke after the task
- [ ] Network egress is limited (no unrestricted outbound from agent sandboxes when available)
- [ ] Changes to tests, `.verify/`, migrations and security config are reviewed in the pull request (no CI label gate)
- [ ] Outbound actions (publish, deploy, email, payment) need explicit human approval
- [ ] Verification uses `tekmor verify --` with allowlisted commands only for learning credit
