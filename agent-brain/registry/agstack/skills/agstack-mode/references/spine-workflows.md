# Spine Workflows & Autonomous Workflow Orchestration

`agstack` pairs with declarative DAG workflows (`workflows/*.yaml`) to orchestrate multi-step, multi-agent operations with checkpoint gates, verification checks, and human approval gates.

**Dual-Mode Execution:** If `agent-spine` is installed, workflows can be executed via the `agent-spine` binary or the `spine_submit_workflow` MCP tool. If `agent-spine` is NOT present, any coding agent directly acts as the workflow engine via the `workflow-engine` skill, or developers can invoke the portable `run-workflow.sh` runner.

---

## 1. Available Registry Workflows

The central registry provides curated DAG workflows under `workflows/` that automate `agstack` playbooks:

| Workflow | Alias | Playbook | Purpose |
|---|---|---|---|
| `workflows/bugfix.yaml` | `@bugfix` | `bug-fix` | Reproduce (failing test) -> Isolate (root cause) -> Fix (immutable) -> Verify (.verify/) -> QualityGate -> Review. |
| `workflows/feature.yaml` | `@feature` | `feature` | ContextAndIntent -> ModelDomain -> FeatureContract -> Implement -> VerificationSuites -> TestRun -> QualityGate -> SecurityGate -> ReviewGate -> Summary. |
| `workflows/stacked-pr.yaml` | `@stacked-pr` | `pull-request` | InspectStack -> AtomicChunking -> VerificationGate -> RichDescription (HLD/LLD) -> ApprovalGate -> PushBranch. |
| `workflows/release-notes.yaml` | `@release-notes` | `technical-writing` | CollectDiff -> DraftNotes (Diátaxis) -> ReviewGate -> Publish. |
| `workflows/mcp-health-check.yaml` | `@mcp-health-check` | `investigation` | AuditSchemas -> ProbeGateway -> VerifyRouting -> Report. |

---

## 2. Invocation Methods

### Method A: Spine CLI Execution (When agent-spine is installed)
```bash
# Initialize workflow template into repository
agent-spine init --with @bugfix

# Execute workflow directly
agent-spine run workflows/bugfix.yaml

# Run with custom payload parameters
agent-spine run workflows/feature.yaml --payload '{"repo_root":".","feature":"billing-invoices"}'
```

### Method B: Spine MCP Tool Invocation (When agent-body MCP is present)
```json
// Submit workflow
call_mcp_tool(
  ServerName: "agent-body",
  ToolName: "spine_submit_workflow",
  Arguments: { "workflow": "bugfix", "payload": { "feature": "billing" } }
)

// Check progress
call_mcp_tool(
  ServerName: "agent-body",
  ToolName: "spine_check_status",
  Arguments: { "workflow_id": "<workflow-id>" }
)
```

### Method C: Autonomous Agent Execution (When agent-spine is absent)
Any coding agent (Antigravity, Cursor, Claude Code, OpenCode) executes the workflow directly via the `workflow-engine` skill:
1. Parse `start_node`, `nodes`, and `edges` from `workflows/<workflow>.yaml`.
2. Step through each node along directed edges.
3. For `kind: agent`: Execute the task specified in `description`.
4. For `kind: checkpoint` / `verify`: Execute the verification script (`.verify/run.sh`, `check-added-comments.sh`). Require exit 0.
5. For `kind: approval_gate`: Present HLD/LLD diagrams and evidence; halt and request human approval before advancing.

### Method D: Portable Standalone Runner (`run-workflow.sh`)
```bash
# Validate workflow DAG
./packages/agstack/scripts/run-workflow.sh validate workflows/feature.yaml

# Inspect workflow DAG flow
./packages/agstack/scripts/run-workflow.sh inspect workflows/feature.yaml

# Execute workflow step-by-step
./packages/agstack/scripts/run-workflow.sh run workflows/bugfix.yaml
```

---

## 3. Node Invariants in Workflows

- **`kind: agent`:** Autonomous execution step (e.g. writing code, reproducing bug).
- **`kind: verify` / `checkpoint`:** Hard automated gate (e.g. executing `.verify/run.sh <feature>`). A failing gate stops the workflow immediately; never weak tests to pass a gate.
- **`kind: approval_gate`:** Pauses execution for human review and sign-off before consequential actions (e.g., merging, deploying, pushing release notes).
