#!/usr/bin/env python3

import sys
import os
import json
import argparse
import subprocess

try:
    import yaml
except ImportError:
    yaml = None


def load_yaml(path):
    if not os.path.exists(path):
        print(f"Error: file not found: {path}", file=sys.stderr)
        sys.exit(1)
    
    with open(path, "r", encoding="utf-8") as f:
        content = f.read()

    if yaml is not None:
        return yaml.safe_load(content)

    data = {"nodes": [], "edges": []}
    lines = content.splitlines()
    section = None
    current_node = None
    current_edge = None

    for raw_line in lines:
        line = raw_line.strip()
        if not line or line.startswith("#"):
            continue

        if line.startswith("name:") and section is None:
            data["name"] = line.split(":", 1)[1].strip().strip('"\'')
        elif line.startswith("start_node:"):
            data["start_node"] = line.split(":", 1)[1].strip().strip('"\'')
        elif line.startswith("nodes:"):
            section = "nodes"
        elif line.startswith("edges:"):
            section = "edges"
        elif section == "nodes":
            if line.startswith("- name:"):
                current_node = {"name": line.split(":", 1)[1].strip().strip('"\'')}
                data["nodes"].append(current_node)
            elif current_node:
                if line.startswith("kind:"):
                    current_node["kind"] = line.split(":", 1)[1].strip().strip('"\'')
                elif line.startswith("description:"):
                    current_node["description"] = line.split(":", 1)[1].strip().strip('"\'')
        elif section == "edges":
            if line.startswith("- from:"):
                current_edge = {"from": line.split(":", 1)[1].strip().strip('"\'')}
                data["edges"].append(current_edge)
            elif current_edge and line.startswith("to:"):
                current_edge["to"] = line.split(":", 1)[1].strip().strip('"\'')

    return data


def validate_workflow(data, filename="workflow"):
    errors = []
    if "name" not in data:
        errors.append("Missing required top-level field: 'name'")
    if "start_node" not in data:
        errors.append("Missing required top-level field: 'start_node'")
    
    nodes = {n.get("name"): n for n in data.get("nodes", []) if isinstance(n, dict)}
    if not nodes:
        errors.append("Workflow has no nodes defined under 'nodes'")
    
    start_node = data.get("start_node")
    if start_node and start_node not in nodes:
        errors.append(f"start_node '{start_node}' is not defined in nodes list")

    edges = data.get("edges", [])
    for idx, edge in enumerate(edges):
        src = edge.get("from")
        dst = edge.get("to")
        if not src or src not in nodes:
            errors.append(f"Edge #{idx} source '{src}' does not exist in nodes")
        if not dst or dst not in nodes:
            errors.append(f"Edge #{idx} destination '{dst}' does not exist in nodes")

    if errors:
        print(f"Validation FAILED for {filename}:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return False

    print(f"PASS: Workflow '{data.get('name')}' is valid ({len(nodes)} nodes, {len(edges)} edges, start: {start_node}).")
    return True


def inspect_workflow(data, filename="workflow"):
    print(f"\n========================================================")
    print(f"Workflow: {data.get('name', 'Unnamed')}")
    print(f"Start Node: {data.get('start_node')}")
    print(f"Total Nodes: {len(data.get('nodes', []))}")
    print(f"Total Edges: {len(data.get('edges', []))}")
    print(f"========================================================\n")

    nodes = {n.get("name"): n for n in data.get("nodes", [])}
    print("Nodes:")
    for n in data.get("nodes", []):
        kind = n.get("kind", "agent")
        desc = n.get("description", "").strip()
        print(f"  * [{kind.upper()}] {n.get('name')}:")
        if desc:
            print(f"      {desc}")

    print("\nDirected Edge Flow:")
    for e in data.get("edges", []):
        src = e.get("from")
        dst = e.get("to")
        src_kind = nodes.get(src, {}).get("kind", "agent")
        dst_kind = nodes.get(dst, {}).get("kind", "agent")
        print(f"  {src} ({src_kind}) ---> {dst} ({dst_kind})")
    print()


def run_workflow(data, auto_approve=False, state_file=None):
    nodes = {n.get("name"): n for n in data.get("nodes", [])}
    edges = {}
    for e in data.get("edges", []):
        edges[e.get("from")] = e.get("to")

    current = data.get("start_node")
    workflow_name = data.get("name")
    print(f"\n[workflow-engine] Starting workflow '{workflow_name}' at node: {current}")

    history = []
    step_num = 1

    while current:
        node_info = nodes.get(current, {})
        kind = node_info.get("kind", "agent")
        desc = node_info.get("description", "").strip()

        print(f"\n--------------------------------------------------------")
        print(f"Step {step_num}: [{kind.upper()}] {current}")
        print(f"Description: {desc}")
        print(f"--------------------------------------------------------")

        if kind in ("checkpoint", "verify"):
            cmd = None
            if ".verify/run.sh" in desc:
                cmd = "bash .verify/run.sh"
            elif "check-added-comments.sh" in desc:
                cmd = "./packages/agstack/scripts/check-added-comments.sh"
            elif "feature_coverage.sh" in desc:
                cmd = "bash .verify/checks/feature_coverage.sh"

            if cmd:
                print(f"[workflow-engine] Executing automated gate: `{cmd}`")
                res = subprocess.run(cmd, shell=True)
                if res.returncode != 0:
                    print(f"\n[workflow-engine] GATE FAILED with exit code {res.returncode}.", file=sys.stderr)
                    print(f"Resolve the issue and re-run. Workflow halted at {current}.", file=sys.stderr)
                    sys.exit(res.returncode)
                print(f"[workflow-engine] GATE PASSED: exit code 0.")
            else:
                print(f"[workflow-engine] Automated verification checkpoint: manual or agent confirmation required.")
        elif kind == "approval_gate":
            print(f"[workflow-engine] HUMAN APPROVAL REQUIRED for '{current}'.")
            if not auto_approve:
                try:
                    answer = input("Proceed past approval gate? [y/N]: ").strip().lower()
                except EOFError:
                    answer = "n"
                if answer not in ("y", "yes"):
                    print(f"[workflow-engine] Workflow halted by user at approval gate '{current}'.")
                    sys.exit(1)
            else:
                print("[workflow-engine] Auto-approve flag set. Proceeding past approval gate.")

        history.append({
            "step": step_num,
            "node": current,
            "kind": kind,
            "status": "passed"
        })

        if state_file:
            os.makedirs(os.path.dirname(state_file) or ".", exist_ok=True)
            with open(state_file, "w", encoding="utf-8") as sf:
                json.dump({"workflow": workflow_name, "history": history, "last_node": current}, sf, indent=2)

        current = edges.get(current)
        step_num += 1

    print(f"\n[workflow-engine] SUCCESS: Workflow '{workflow_name}' completed all {step_num - 1} steps cleanly.\n")


def main():
    parser = argparse.ArgumentParser(description="Zero-dependency agstack Workflow Engine")
    subparsers = parser.add_subparsers(dest="command", required=True)

    p_val = subparsers.add_parser("validate", help="Validate workflow DAG structure")
    p_val.add_argument("path", help="Path to workflow YAML file")

    p_ins = subparsers.add_parser("inspect", help="Inspect workflow nodes, kinds, and flow")
    p_ins.add_argument("path", help="Path to workflow YAML file")

    p_list = subparsers.add_parser("list", help="List all workflows in directory")
    p_list.add_argument("dir", nargs="?", default="workflows", help="Directory containing workflows (default: workflows)")

    p_run = subparsers.add_parser("run", help="Execute workflow steps")
    p_run.add_argument("path", help="Path to workflow YAML file")
    p_run.add_argument("--auto-approve", action="store_true", help="Auto-approve approval gates")
    p_run.add_argument("--state-file", default=".verify/artifacts/workflow-state.json", help="Path to record state JSON")

    args = parser.parse_args()

    if args.command == "list":
        if not os.path.isdir(args.dir):
            print(f"Directory not found: {args.dir}", file=sys.stderr)
            sys.exit(1)
        files = [os.path.join(args.dir, f) for f in os.listdir(args.dir) if f.endswith((".yaml", ".yml"))]
        print(f"\nWorkflows in '{args.dir}':")
        for f in sorted(files):
            try:
                data = load_yaml(f)
                name = data.get("name", "Unnamed")
                start = data.get("start_node", "?")
                nodes_cnt = len(data.get("nodes", []))
                print(f"  * {os.path.basename(f):<24} - {name} ({nodes_cnt} nodes, start: {start})")
            except Exception as e:
                print(f"  * {os.path.basename(f):<24} - Error parsing: {e}")
        print()
        return

    data = load_yaml(args.path)

    if args.command == "validate":
        valid = validate_workflow(data, args.path)
        sys.exit(0 if valid else 1)
    elif args.command == "inspect":
        inspect_workflow(data, args.path)
    elif args.command == "run":
        if not validate_workflow(data, args.path):
            sys.exit(1)
        run_workflow(data, auto_approve=args.auto_approve, state_file=args.state_file)


if __name__ == "__main__":
    main()
