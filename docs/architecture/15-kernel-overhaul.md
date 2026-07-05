# 15 — Kernel overhaul (V2)

> **Do not commit the full phased implementation tracker to this repository.**

The editable working copy lives locally at:

`docs/superpowers/plans/2026-07-02-kernel-overhaul.md` (gitignored)

Use that file for phase status, deliverables, verification commands, and benchmarks during kernel V2 rollout.

## Summary

Cross-organ phased rollout treating the agent runtime as an OS kernel — specialized organs, deterministic Rust infrastructure, and **only agent-eyes (VLM) and agent-mouth (SLM) call LLM-class models**.

For architecture context (routing, memory, enforcement), see the numbered articles in this directory — not the kernel phase tracker.
