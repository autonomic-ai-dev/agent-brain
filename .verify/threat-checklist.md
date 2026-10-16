# Agent threat checklist (Stage 1 / review finding #3)

Before running agent workflows against a repo:

- [ ] No production credentials in the worktree, Compose files, or `.env` committed to git
- [ ] Tokens/secrets are scoped and short-lived; revoke after the task
- [ ] Network egress is limited (no unrestricted outbound from agent sandboxes when available)
- [ ] Protected paths (tests, `.verify/`, migrations, security config) require human approval / CI label
- [ ] Outbound actions (publish, deploy, email, payment) need explicit human approval
- [ ] Verification uses `agent-brain verify --` with allowlisted commands only for learning credit
