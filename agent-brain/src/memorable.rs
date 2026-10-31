use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{bail, Context, Result};
use regex::Regex;
use serde_json::{json, Value};

use crate::db::store::BrainStore;
use crate::settings::{AgentBrainSettings, UpstreamServerConfig};
use crate::verify::last_verification;

pub const SERVER_NAME: &str = "memorable";

const COMMAND: &str = "memorable";
const UNREACHABLE_API_URL: &str = "http://127.0.0.1:9";
const TARGET_KEYS: [&str; 5] = ["file_path", "path", "command", "pattern", "url"];
const SECRET_TOKEN_PREFIXES: [&str; 4] = ["sk-", "ghp_", "AKIA", "xox"];
const SECRET_SUBSTRINGS: [&str; 2] = ["-----BEGIN", "Bearer "];
const MAX_TARGET_CHARS: usize = 200;
const MAX_PROMPT_CHARS: usize = 2000;
const MAX_TASK_CHARS: usize = 200;
const MIN_TOOL_CALLS: usize = 3;
const MUTATING_TOOLS: [&str; 4] = ["Edit", "MultiEdit", "Write", "NotebookEdit"];
const VERIFY_COMMANDS: [&str; 6] = [
    "cargo test",
    "cargo clippy",
    "agent-brain verify",
    "pytest",
    "npm test",
    "make test",
];

pub fn is_registered(settings: &AgentBrainSettings) -> bool {
    settings
        .upstream_mcp
        .servers
        .iter()
        .any(|server| server.name == SERVER_NAME)
}

pub fn enable(settings: &mut AgentBrainSettings, capture: bool) {
    settings.memorable.enabled = true;
    settings.memorable.capture = capture;
    settings
        .upstream_mcp
        .servers
        .retain(|server| server.name != SERVER_NAME);
    settings.upstream_mcp.servers.push(UpstreamServerConfig {
        name: SERVER_NAME.to_string(),
        command: COMMAND.to_string(),
        args: vec!["mcp".to_string()],
        env: HashMap::from([(
            "MEMORABLE_API_URL".to_string(),
            UNREACHABLE_API_URL.to_string(),
        )]),
        enabled: true,
    });
}

pub fn disable(settings: &mut AgentBrainSettings) {
    settings.memorable.enabled = false;
    settings.memorable.capture = false;
    settings
        .upstream_mcp
        .servers
        .retain(|server| server.name != SERVER_NAME);
}

pub fn capture_allowed(settings: &AgentBrainSettings, last_passing: bool) -> Result<()> {
    if !settings.memorable.enabled || !settings.memorable.capture {
        bail!("memorable capture is off; run `agent-brain memorable enable --capture` to allow it");
    }
    if !last_passing {
        bail!("the last `agent-brain verify` for this repo did not pass; capture only verified sessions");
    }
    Ok(())
}

pub fn reduce_trace(trace: &Value) -> Result<Value> {
    let calls = trace
        .get("tool_calls")
        .and_then(Value::as_array)
        .context("trace needs a tool_calls array")?;
    let reduced: Vec<Value> = calls.iter().map(reduce_call).collect();
    let mut out = json!({
        "session_id": trace.get("session_id").and_then(Value::as_str).unwrap_or("capture"),
        "harness": trace.get("harness").and_then(Value::as_str).unwrap_or("agent-brain"),
        "tool_calls": reduced,
    });
    for key in ["workflow_id", "prompt", "task_description"] {
        if let Some(value) = trace.get(key) {
            out[key] = value.clone();
        }
    }
    Ok(out)
}

fn reduce_call(call: &Value) -> Value {
    let name = call
        .get("name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let input = TARGET_KEYS
        .iter()
        .find_map(|key| {
            let value = call.get("input")?.get(*key)?.as_str()?;
            Some(json!({ *key: value.chars().take(MAX_TARGET_CHARS).collect::<String>() }))
        })
        .unwrap_or_else(|| json!({}));
    let outcome = call.get("result").and_then(|result| {
        result
            .get("exit_code")
            .and_then(Value::as_i64)
            .map(|code| json!({ "exit_code": code }))
            .or_else(|| {
                result
                    .get("ok")
                    .and_then(Value::as_bool)
                    .map(|ok| json!({ "ok": ok }))
            })
    });
    let mut reduced = json!({ "name": name, "input": input });
    if let Some(outcome) = outcome {
        reduced["result"] = outcome;
    }
    reduced
}

fn record_outcomes(workflow: &mut Value, blocks: &[Value]) {
    let Some(calls) = workflow["tool_calls"].as_array_mut() else {
        return;
    };
    for block in blocks.iter().filter(|block| block["type"] == "tool_result") {
        let Some(is_error) = block["is_error"].as_bool() else {
            continue;
        };
        if let Some(call) = calls
            .iter_mut()
            .find(|call| call["id"] == block["tool_use_id"])
        {
            call["result"] = json!({ "ok": !is_error });
        }
    }
}

pub fn secret_marker(text: &str) -> Option<&'static str> {
    SECRET_SUBSTRINGS
        .into_iter()
        .find(|marker| text.contains(marker))
        .or_else(|| {
            text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                .find_map(|token| {
                    SECRET_TOKEN_PREFIXES
                        .into_iter()
                        .find(|prefix| token.starts_with(prefix))
                })
        })
}

pub fn capture(home: &Path, db_path: &Path, cwd: &Path, trace_path: &Path) -> Result<String> {
    let trace = read_json(trace_path)?;
    capture_traces(home, db_path, cwd, std::slice::from_ref(&trace))
}

pub fn capture_transcript(
    home: &Path,
    db_path: &Path,
    cwd: &Path,
    transcript_path: &Path,
) -> Result<String> {
    let raw = std::fs::read_to_string(transcript_path)
        .with_context(|| format!("read {}", transcript_path.display()))?;
    let repo = cwd.to_string_lossy();
    let mut traces = workflows_from_transcript(&raw, &repo);
    let found = traces.len();
    traces.retain(is_strong_workflow);
    let skipped = found - traces.len();
    let message = capture_traces(home, db_path, cwd, &traces)?;
    Ok(format!("{message}; skipped {skipped} weak workflow(s)"))
}

fn is_strong_workflow(workflow: &Value) -> bool {
    let Some(calls) = workflow["tool_calls"].as_array() else {
        return false;
    };
    let changes_something = calls
        .iter()
        .any(|call| MUTATING_TOOLS.iter().any(|tool| call["name"] == *tool));
    calls.len() >= MIN_TOOL_CALLS
        && changes_something
        && calls.iter().any(|call| {
            call["name"] == "Bash"
                && call["result"]["ok"] == true
                && call["input"]["command"].as_str().is_some_and(|command| {
                    VERIFY_COMMANDS
                        .iter()
                        .any(|verify| command.contains(verify))
                })
        })
}

fn read_json(path: &Path) -> Result<Value> {
    let raw = std::fs::read_to_string(path).with_context(|| format!("read {}", path.display()))?;
    serde_json::from_str(&raw).context("trace is not valid JSON")
}

fn capture_traces(home: &Path, db_path: &Path, cwd: &Path, traces: &[Value]) -> Result<String> {
    let project = crate::project_key::project_key_if_known(cwd)
        .context("capture needs a repo with an agent-brain project key")?;
    let store = BrainStore::open(db_path)?;
    let last_passing = last_verification(&store, &project).is_some_and(|status| status.passing);
    let settings = AgentBrainSettings::load(home);
    capture_allowed(&settings, last_passing)?;

    let payloads = traces
        .iter()
        .map(|trace| serde_json::to_string(&reduce_trace(trace)?).map_err(Into::into))
        .collect::<Result<Vec<String>>>()?;
    let outbox = outbox_path(home);
    std::fs::create_dir_all(outbox.parent().context("outbox has a parent")?)?;
    let mut ingested = 0;
    let mut withheld = Vec::new();
    for (index, payload) in payloads.iter().enumerate() {
        if let Some(marker) = secret_marker(payload) {
            withheld.push(format!("{} (marker `{marker}`)", index + 1));
            continue;
        }
        std::fs::write(&outbox, format!("{payload}\n"))
            .with_context(|| format!("write {}", outbox.display()))?;
        let output = Command::new(COMMAND)
            .arg("ingest")
            .arg(&outbox)
            .output()
            .context("run memorable ingest (is the memorable binary on PATH?)")?;
        let _ = std::fs::remove_file(&outbox);
        if !output.status.success() {
            bail!(
                "memorable ingest failed after {ingested} of {}: {}",
                payloads.len(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        ingested += 1;
    }
    if withheld.is_empty() {
        Ok(format!("ingested {ingested} workflow(s)"))
    } else {
        Ok(format!(
            "ingested {ingested} of {} workflow(s); withheld for credential-shaped values: {}",
            payloads.len(),
            withheld.join(", ")
        ))
    }
}

fn task_line(prompt: &str) -> String {
    let line = prompt
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('<') && !line.starts_with('/'))
        .unwrap_or_else(|| prompt.trim());
    scrub(line).chars().take(MAX_TASK_CHARS).collect()
}

fn scrub(text: &str) -> String {
    let email =
        Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("valid email pattern");
    let without_home = match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => text.replace(&home, "~"),
        _ => text.to_string(),
    };
    email.replace_all(&without_home, "<email>").into_owned()
}

pub fn workflows_from_transcript(raw: &str, repo: &str) -> Vec<Value> {
    let mut workflows: Vec<Value> = Vec::new();
    for record in raw
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
    {
        let in_repo = record
            .get("cwd")
            .and_then(Value::as_str)
            .is_some_and(|cwd| cwd == repo || cwd.starts_with(&format!("{repo}/")));
        if !in_repo {
            continue;
        }
        match record.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let (Some(workflow), Some(blocks)) = (
                    workflows.last_mut(),
                    record.pointer("/message/content").and_then(Value::as_array),
                ) {
                    record_outcomes(workflow, blocks);
                }
                let is_meta = record
                    .get("isMeta")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let prompt = record
                    .pointer("/message/content")
                    .and_then(Value::as_str)
                    .filter(|text| !is_meta && !text.starts_with('<'));
                if let Some(prompt) = prompt {
                    workflows.push(json!({
                        "session_id": record.get("sessionId"),
                        "workflow_id": record.get("uuid"),
                        "harness": "claude-code",
                        "prompt": scrub(prompt).chars().take(MAX_PROMPT_CHARS).collect::<String>(),
                        "task_description": task_line(prompt),
                        "tool_calls": [],
                    }));
                }
            }
            Some("assistant") => {
                let blocks = record.pointer("/message/content").and_then(Value::as_array);
                if let (Some(workflow), Some(blocks)) = (workflows.last_mut(), blocks) {
                    let calls = blocks
                        .iter()
                        .filter(|block| block["type"] == "tool_use")
                        .map(|block| {
                            json!({ "id": block["id"], "name": block["name"], "input": block["input"] })
                        });
                    if let Some(tool_calls) = workflow["tool_calls"].as_array_mut() {
                        tool_calls.extend(calls);
                    }
                }
            }
            _ => {}
        }
    }
    workflows.retain(|workflow| !workflow["tool_calls"].as_array().is_some_and(Vec::is_empty));
    workflows
}

fn outbox_path(home: &Path) -> PathBuf {
    home.join("memorable").join("outbox.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace() -> Value {
        json!({
            "session_id": "s1",
            "title": "private task title",
            "tool_calls": [
                {"name": "Edit", "input": {"file_path": "src/lib.rs", "old_string": "secret body"}, "result": {"ok": true, "stdout": "file contents"}},
                {"name": "Bash", "input": {"command": "cargo test"}, "result": {"exit_code": 0, "stdout": "lots of output"}}
            ]
        })
    }

    #[test]
    fn reduce_keeps_only_name_target_and_exit_code() {
        let reduced = reduce_trace(&sample_trace()).unwrap();
        let text = reduced.to_string();
        assert!(!text.contains("private task title"));
        assert!(!text.contains("secret body"));
        assert!(!text.contains("file contents"));
        assert!(!text.contains("lots of output"));
        assert_eq!(reduced["tool_calls"][0]["input"]["file_path"], "src/lib.rs");
        assert_eq!(reduced["tool_calls"][1]["result"]["exit_code"], 0);
    }

    #[test]
    fn reduce_requires_tool_calls() {
        assert!(reduce_trace(&json!({"session_id": "s"})).is_err());
    }

    #[test]
    fn reduce_keeps_the_prompt_and_omits_an_unknown_outcome() {
        let trace = json!({
            "session_id": "s2",
            "harness": "claude-code",
            "prompt": "fix the parser",
            "task_description": "fix the parser",
            "tool_calls": [{"name": "Read", "input": {"file_path": "src/lib.rs"}}]
        });
        let reduced = reduce_trace(&trace).unwrap();
        assert_eq!(reduced["prompt"], "fix the parser");
        assert_eq!(reduced["harness"], "claude-code");
        assert!(reduced["tool_calls"][0].get("result").is_none());
    }

    #[test]
    fn transcript_becomes_one_workflow_per_human_prompt_in_this_repo() {
        let raw = [
            r#"{"type":"user","cwd":"/repo","sessionId":"s","uuid":"u1","message":{"content":"add a test"}}"#,
            r#"{"type":"assistant","cwd":"/repo","sessionId":"s","message":{"content":[{"type":"text","text":"ok"},{"type":"tool_use","name":"Edit","input":{"file_path":"src/a.rs"}}]}}"#,
            r#"{"type":"user","cwd":"/repo","sessionId":"s","isMeta":true,"message":{"content":"meta"}}"#,
            r#"{"type":"user","cwd":"/repo","sessionId":"s","uuid":"u3","message":{"content":"<command-name>/model</command-name>"}}"#,
            r#"{"type":"user","cwd":"/repo2","sessionId":"s","uuid":"u2","message":{"content":"sibling repo"}}"#,
            r#"{"type":"assistant","cwd":"/repo2","sessionId":"s","message":{"content":[{"type":"tool_use","name":"Bash","input":{"command":"ls"}}]}}"#,
            r#"{"type":"user","cwd":"/repo","sessionId":"s","uuid":"u4","message":{"content":"no tools here"}}"#,
        ]
        .join("\n");
        let workflows = workflows_from_transcript(&raw, "/repo");
        assert_eq!(workflows.len(), 1);
        assert_eq!(workflows[0]["prompt"], "add a test");
        assert_eq!(workflows[0]["workflow_id"], "u1");
        assert_eq!(workflows[0]["tool_calls"][0]["name"], "Edit");
    }

    #[test]
    fn credential_shapes_are_detected() {
        assert_eq!(secret_marker("token sk-abc123"), Some("sk-"));
        assert_eq!(
            secret_marker("-----BEGIN PRIVATE KEY-----"),
            Some("-----BEGIN")
        );
        assert_eq!(secret_marker("cargo test --lib"), None);
    }

    #[test]
    fn transcript_outcomes_come_only_from_explicit_error_flags() {
        let raw = [
            r#"{"type":"user","cwd":"/repo","sessionId":"s","uuid":"u1","message":{"content":"run the tests"}}"#,
            r#"{"type":"assistant","cwd":"/repo","sessionId":"s","message":{"content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"cargo test"}},{"type":"tool_use","id":"t2","name":"Read","input":{"file_path":"src/a.rs"}}]}}"#,
            r#"{"type":"user","cwd":"/repo","sessionId":"s","message":{"content":[{"type":"tool_result","tool_use_id":"t1","is_error":true,"content":"failed output"},{"type":"tool_result","tool_use_id":"t2","content":"file body"}]}}"#,
        ]
        .join("\n");
        let workflows = workflows_from_transcript(&raw, "/repo");
        let reduced = reduce_trace(&workflows[0]).unwrap();
        assert_eq!(reduced["tool_calls"][0]["result"]["ok"], false);
        assert!(reduced["tool_calls"][1].get("result").is_none());
        assert!(reduced["tool_calls"][0].get("id").is_none());
        assert!(!reduced.to_string().contains("failed output"));
        assert!(!reduced.to_string().contains("file body"));
    }

    #[test]
    fn task_line_is_the_first_substantive_line_with_emails_scrubbed() {
        let prompt = "<command-name>/model</command-name>\n\n  \nEmail jane.doe@example.com the release notes\nand then run the tests";
        assert_eq!(task_line(prompt), "Email <email> the release notes");
    }

    fn workflow_with(calls: Value) -> Value {
        json!({ "tool_calls": calls })
    }

    fn edit() -> Value {
        json!({ "name": "Edit", "input": { "file_path": "src/a.rs" }, "result": { "ok": true } })
    }

    fn verified_test_run(ok: bool) -> Value {
        json!({ "name": "Bash", "input": { "command": "cargo test -p agent-brain" }, "result": { "ok": ok } })
    }

    #[test]
    fn strong_workflow_has_enough_steps_and_a_passing_verifier_run() {
        let workflow = workflow_with(json!([edit(), edit(), verified_test_run(true)]));
        assert!(is_strong_workflow(&workflow));
    }

    #[test]
    fn read_only_workflow_is_weak_even_with_a_passing_verifier() {
        let read = json!({ "name": "Read", "input": { "file_path": "src/a.rs" }, "result": { "ok": true } });
        let workflow = workflow_with(json!([read.clone(), read, verified_test_run(true)]));
        assert!(!is_strong_workflow(&workflow));
    }

    #[test]
    fn workflow_with_too_few_steps_is_weak() {
        let workflow = workflow_with(json!([edit(), verified_test_run(true)]));
        assert!(!is_strong_workflow(&workflow));
    }

    #[test]
    fn workflow_whose_verifier_failed_or_never_ran_is_weak() {
        let failed = workflow_with(json!([edit(), edit(), verified_test_run(false)]));
        let never_verified = workflow_with(json!([edit(), edit(), edit()]));
        assert!(!is_strong_workflow(&failed));
        assert!(!is_strong_workflow(&never_verified));
    }

    #[test]
    fn ordinary_words_containing_a_marker_are_not_credentials() {
        assert_eq!(secret_marker("rewrite the task-runner and the asks"), None);
        assert_eq!(secret_marker("export KEY=AKIA1234"), Some("AKIA"));
    }

    #[test]
    fn capture_needs_both_flags_and_a_passing_verify() {
        let mut settings = AgentBrainSettings::default();
        assert!(capture_allowed(&settings, true).is_err());
        enable(&mut settings, false);
        assert!(capture_allowed(&settings, true).is_err());
        enable(&mut settings, true);
        assert!(capture_allowed(&settings, false).is_err());
        assert!(capture_allowed(&settings, true).is_ok());
    }

    #[test]
    fn enable_without_capture_leaves_capture_off() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        assert!(settings.memorable.enabled);
        assert!(!settings.memorable.capture);
    }

    #[test]
    fn memorable_is_off_by_default() {
        let settings = AgentBrainSettings::default();
        assert!(!settings.memorable.enabled);
        assert!(!settings.memorable.capture);
        assert!(!is_registered(&settings));
    }

    #[test]
    fn enable_registers_a_read_only_server_pointed_at_an_unreachable_api() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        let server = settings
            .upstream_mcp
            .servers
            .iter()
            .find(|s| s.name == SERVER_NAME)
            .unwrap();
        assert_eq!(server.command, "memorable");
        assert_eq!(server.args, vec!["mcp".to_string()]);
        assert_eq!(
            server.env.get("MEMORABLE_API_URL").map(String::as_str),
            Some(UNREACHABLE_API_URL)
        );
    }

    #[test]
    fn enable_twice_registers_one_server() {
        let mut settings = AgentBrainSettings::default();
        enable(&mut settings, false);
        enable(&mut settings, true);
        assert_eq!(
            settings
                .upstream_mcp
                .servers
                .iter()
                .filter(|s| s.name == SERVER_NAME)
                .count(),
            1
        );
    }

    #[test]
    fn disable_clears_capture_and_keeps_other_servers() {
        let mut settings = AgentBrainSettings::default();
        settings.upstream_mcp.servers.push(UpstreamServerConfig {
            name: "stitch".to_string(),
            command: "npx".to_string(),
            args: Vec::new(),
            env: HashMap::new(),
            enabled: true,
        });
        enable(&mut settings, true);
        disable(&mut settings);
        assert!(!settings.memorable.enabled);
        assert!(!settings.memorable.capture);
        assert!(!is_registered(&settings));
        assert!(settings
            .upstream_mcp
            .servers
            .iter()
            .any(|s| s.name == "stitch"));
    }
}
