#!/usr/bin/env python3
"""GAP-MET-01: OpenCode + agent-brain integration bench.

Checks registry bootstrap, cache sync, cross-host agent-brain mode files,
and optional route latency (``agent-brain bench --ci``).

Usage:
  python3 scripts/opencode_integration_bench.py
  python3 scripts/opencode_integration_bench.py --json
  python3 scripts/opencode_integration_bench.py --full
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import statistics
import subprocess
import sys
import time
from dataclasses import asdict, dataclass
from pathlib import Path
from typing import Optional

AGENT_BRAIN = os.environ.get("AGENT_BRAIN_BIN", "agent-brain")
HOME = Path(os.environ.get("AGENT_BRAIN_HOME", Path.home() / ".agent_brain"))

MODE_CANDIDATES: list[tuple[str, Path]] = [
    ("cursor_project", Path(".cursor/rules/agent-brain-mode.mdc")),
    ("codex", Path.home() / ".codex" / "agent-brain-mode.md"),
    ("gemini", Path.home() / ".gemini" / "agent-brain-mode.md"),
    ("opencode", Path.home() / ".config/opencode/modes/agent-brain.md"),
    ("claude_code", Path.home() / ".claude" / "agent-brain-mode.md"),
    ("vscode", Path.home() / ".vscode/agent-brain-mode.md"),
]

REQUIRED_ALIASES = ("supervisor", "starter")
REQUIRED_WORKFLOWS = ("release-notes", "stacked-pr", "bugfix")


@dataclass
class Check:
    id: str
    passed: bool
    detail: str
    ms: Optional[float] = None


def run(cmd: list[str], timeout: int = 120) -> tuple[subprocess.CompletedProcess[str], float]:
    start = time.perf_counter()
    proc = subprocess.run(
        cmd,
        capture_output=True,
        text=True,
        timeout=timeout,
        check=False,
    )
    ms = (time.perf_counter() - start) * 1000.0
    return proc, ms


def check_binary() -> Check:
    path = shutil.which(AGENT_BRAIN)
    ok = path is not None
    return Check("agent_brain.on_path", ok, path or f"{AGENT_BRAIN} not found")


def check_registry_list() -> Check:
    proc, ms = run([AGENT_BRAIN, "registry", "list"])
    text = proc.stdout
    missing: list[str] = []
    for alias in REQUIRED_ALIASES:
        if alias not in text:
            missing.append(f"@{alias}")
    for wf in REQUIRED_WORKFLOWS:
        if wf not in text:
            missing.append(f"workflow:{wf}")
    ok = proc.returncode == 0 and not missing
    detail = "ok" if ok else f"missing {', '.join(missing)}; rc={proc.returncode}"
    return Check("registry.list", ok, detail, ms)


def check_registry_sync_local() -> Check:
    proc, ms = run([AGENT_BRAIN, "registry", "sync", "--local"])
    manifest = HOME / "registry-cache" / "manifest.json"
    workflows = HOME / "registry-cache" / "workflows" / "release-notes.yaml"
    ok = proc.returncode == 0 and manifest.is_file() and workflows.is_file()
    detail = (
        f"cache at {manifest.parent}"
        if ok
        else (proc.stderr.strip() or f"missing {manifest}")
    )
    return Check("registry.sync_local", ok, detail, ms)


def check_mode_files() -> Check:
    found = [name for name, path in MODE_CANDIDATES if path.is_file()]
    ok = len(found) >= 1
    return Check(
        "host.mode_presence",
        ok,
        f"found {len(found)}: {', '.join(found) if found else 'none'}",
    )


def check_doctor_opencode() -> Check:
    proc, ms = run([AGENT_BRAIN, "doctor"], timeout=180)
    line = next(
        (ln.strip() for ln in proc.stdout.splitlines() if "opencode" in ln.lower()),
        "",
    )
    if not line:
        line = proc.stderr.strip()[:240] or f"doctor rc={proc.returncode}"
    ok = proc.returncode == 0 or "not configured" in line.lower()
    return Check("doctor.opencode_line", ok, line, ms)


def check_route_bench_ci() -> Check:
    times: list[float] = []
    for _ in range(3):
        proc, ms = run([AGENT_BRAIN, "bench", "--ci"], timeout=300)
        if proc.returncode != 0:
            return Check(
                "route.bench_ci",
                False,
                proc.stderr.strip()[:240] or f"rc={proc.returncode}",
                ms,
            )
        times.append(ms)
    p95 = statistics.quantiles(times, n=20)[18] if len(times) >= 5 else max(times)
    ok = p95 <= 45_000
    return Check("route.bench_ci", ok, f"p95={p95:.0f}ms ({len(times)} runs)", p95)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--json", action="store_true", help="emit JSON report")
    parser.add_argument(
        "--full",
        action="store_true",
        help="include agent-brain bench --ci latency gate",
    )
    args = parser.parse_args()

    checks: list[Check] = [
        check_binary(),
        check_registry_list(),
        check_registry_sync_local(),
        check_mode_files(),
        check_doctor_opencode(),
    ]
    if args.full and checks[0].passed:
        checks.append(check_route_bench_ci())

    passed = sum(1 for c in checks if c.passed)
    report = {
        "agent_brain": AGENT_BRAIN,
        "home": str(HOME),
        "passed": passed,
        "total": len(checks),
        "score_pct": round(100.0 * passed / len(checks), 1) if checks else 0,
        "checks": [asdict(c) for c in checks],
    }

    if args.json:
        print(json.dumps(report, indent=2))
    else:
        print(f"OpenCode integration bench: {passed}/{len(checks)} passed")
        for check in checks:
            mark = "PASS" if check.passed else "FAIL"
            timing = f" ({check.ms:.0f}ms)" if check.ms is not None else ""
            print(f"  [{mark}] {check.id}{timing}: {check.detail}")

    return 0 if passed == len(checks) else 1


if __name__ == "__main__":
    sys.exit(main())
