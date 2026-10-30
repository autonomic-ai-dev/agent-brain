use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::AgentBrainSettings;
use crate::types::MustApply;

const TOPIC: &str = "feedback-report";
const STATE_FILE: &str = "state.json";

#[derive(Debug, Default, Serialize, Deserialize)]
struct ReportState {
    last_run: i64,
    pending: Option<PendingReport>,
}

#[derive(Debug, Serialize, Deserialize)]
struct PendingReport {
    telemetry: PathBuf,
    agent: PathBuf,
}

pub fn route_request(home: &Path, logs_dir: &Path, now: i64) -> Option<MustApply> {
    let settings = AgentBrainSettings::load(home).feedback_report;
    if !settings.enabled {
        return None;
    }
    let reports = home.join("reports");
    let interval = settings.interval_hours as i64 * 3600;
    let mut state = load_state(&reports);
    let answered = state
        .pending
        .as_ref()
        .is_some_and(|pending| pending.agent.exists());
    if answered {
        state.pending = None;
    }
    let mut changed = answered;
    if state.pending.is_none() && now - state.last_run >= interval {
        match write_telemetry(home, logs_dir, &reports, now - interval, now) {
            Ok(telemetry) => {
                state.last_run = now;
                state.pending = Some(PendingReport {
                    telemetry,
                    agent: reports.join(format!("agent-feedback-{now}.md")),
                });
                changed = true;
            }
            Err(err) => {
                tracing::warn!(error = %err, "feedback telemetry report not written");
                return None;
            }
        }
    }
    if changed {
        if let Err(err) = save_state(&reports, &state) {
            tracing::warn!(error = %err, "feedback report state not saved");
        }
    }
    state.pending.as_ref().map(|pending| MustApply {
        topic: TOPIC.into(),
        text: request_text(&pending.telemetry, &pending.agent),
    })
}

fn request_text(telemetry: &Path, agent: &Path) -> String {
    format!(
        "Feedback report due. Read the telemetry at {} first, then write your feedback on agent-brain and agent-spine to {} as markdown: what helped, what was wrong or noisy, gate blocks that were false positives, missing capabilities, and memory quality. If agstack-mode or another agstack skill was listed but not applied, say why: for example, it was not read, it ranked low, or it conflicted with the repository harness. This request stays open until that file exists.",
        telemetry.display(),
        agent.display(),
    )
}

fn write_telemetry(
    home: &Path,
    logs_dir: &Path,
    reports: &Path,
    start: i64,
    end: i64,
) -> Result<PathBuf> {
    let gates = window_records(&logs_dir.join("gate-events.jsonl"), "ts", 1, start, end);
    let hooks = window_records(
        &home.join("hooks/hook_events.jsonl"),
        "timestamp",
        1000,
        start,
        end,
    );
    let tools = window_records(
        &home.join("hooks/tool_events.jsonl"),
        "timestamp",
        1000,
        start,
        end,
    );
    let body = telemetry_markdown(start, end, &gates, &hooks, &tools);
    fs::create_dir_all(reports).context("create reports dir")?;
    let path = reports.join(format!("telemetry-{end}.md"));
    fs::write(&path, body).context("write telemetry report")?;
    Ok(path)
}

fn window_records(path: &Path, time_key: &str, scale: i64, start: i64, end: i64) -> Vec<Value> {
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|record| {
            record[time_key]
                .as_i64()
                .map(|time| time / scale)
                .is_some_and(|time| time >= start && time <= end)
        })
        .collect()
}

fn telemetry_markdown(
    start: i64,
    end: i64,
    gates: &[Value],
    hooks: &[Value],
    tools: &[Value],
) -> String {
    let sessions: HashSet<&str> = hooks
        .iter()
        .filter_map(|record| record["session"].as_str())
        .collect();
    let mut out = format!(
        "# agent-brain telemetry report\n\nWindow: {start} to {end} (unix seconds)\n\nSessions seen: {}\n",
        sessions.len(),
    );
    section(
        &mut out,
        "Hook events",
        "event",
        tally(hooks.iter().map(|record| text_field(record, "event"))),
    );
    section(
        &mut out,
        "Gate denials and warnings",
        "kind: detail",
        tally(gates.iter().map(|record| {
            format!(
                "{}: {}",
                text_field(record, "kind"),
                text_field(record, "detail")
            )
        })),
    );
    section(
        &mut out,
        "Tool calls",
        "tool",
        tally(tools.iter().map(|record| text_field(record, "tool_name"))),
    );
    out
}

fn text_field(record: &Value, key: &str) -> String {
    record[key].as_str().unwrap_or("unknown").to_string()
}

fn tally(keys: impl Iterator<Item = String>) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for key in keys {
        *counts.entry(key).or_insert(0) += 1;
    }
    counts
}

fn section(out: &mut String, title: &str, column: &str, counts: BTreeMap<String, usize>) {
    out.push_str(&format!(
        "\n## {title}\n\n| {column} | count |\n| --- | ---: |\n"
    ));
    let mut rows: Vec<(String, usize)> = counts.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    if rows.is_empty() {
        out.push_str("| none | 0 |\n");
    }
    for (key, count) in rows {
        out.push_str(&format!("| {} | {count} |\n", key.replace('|', "\\|")));
    }
}

fn load_state(reports: &Path) -> ReportState {
    fs::read_to_string(reports.join(STATE_FILE))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn save_state(reports: &Path, state: &ReportState) -> Result<()> {
    fs::create_dir_all(reports).context("create reports dir")?;
    let text = serde_json::to_string_pretty(state).context("serialize report state")?;
    fs::write(reports.join(STATE_FILE), text).context("write report state")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_800_000_000;

    fn enabled_home(interval_hours: u64) -> tempfile::TempDir {
        let home = tempfile::tempdir().expect("tempdir");
        let config =
            format!("feedback_report:\n  enabled: true\n  interval_hours: {interval_hours}\n");
        fs::write(home.path().join("config.yaml"), config).expect("write config");
        home
    }

    #[test]
    fn disabled_flag_writes_nothing_and_asks_nothing() {
        let home = tempfile::tempdir().expect("tempdir");
        let logs = home.path().join("logs");

        assert!(route_request(home.path(), &logs, NOW).is_none());
        assert!(!home.path().join("reports").exists());
    }

    #[test]
    fn due_report_writes_telemetry_and_asks_agent_until_answered() {
        let home = enabled_home(3);
        let logs = home.path().join("logs");
        let reports = home.path().join("reports");

        let first = route_request(home.path(), &logs, NOW).expect("request when due");
        assert_eq!(first.topic, TOPIC);
        assert!(reports.join(format!("telemetry-{NOW}.md")).exists());
        let agent_file = reports.join(format!("agent-feedback-{NOW}.md"));
        assert!(first.text.contains(&agent_file.display().to_string()));

        let repeated = route_request(home.path(), &logs, NOW + 60).expect("request stays open");
        assert_eq!(repeated.text, first.text);

        fs::write(&agent_file, "# feedback").expect("write agent feedback");
        assert!(route_request(home.path(), &logs, NOW + 120).is_none());
    }

    #[test]
    fn next_report_waits_for_interval_after_last_run() {
        let home = enabled_home(3);
        let logs = home.path().join("logs");
        let reports = home.path().join("reports");

        route_request(home.path(), &logs, NOW).expect("first request");
        fs::write(
            reports.join(format!("agent-feedback-{NOW}.md")),
            "# feedback",
        )
        .expect("answer");
        assert!(route_request(home.path(), &logs, NOW + 3 * 3600 - 1).is_none());

        let next = route_request(home.path(), &logs, NOW + 3 * 3600).expect("due again");
        assert!(next
            .text
            .contains(&format!("telemetry-{}.md", NOW + 3 * 3600)));
    }

    #[test]
    fn telemetry_counts_only_records_inside_window() {
        let home = enabled_home(3);
        let logs = home.path().join("logs");
        fs::create_dir_all(&logs).expect("logs dir");
        fs::create_dir_all(home.path().join("hooks")).expect("hooks dir");
        let gate_in = format!(
            "{{\"ts\":{},\"kind\":\"bash-block\",\"detail\":\"force push\"}}",
            NOW - 10
        );
        let gate_out = format!(
            "{{\"ts\":{},\"kind\":\"bash-block\",\"detail\":\"old\"}}",
            NOW - 4 * 3600
        );
        fs::write(
            logs.join("gate-events.jsonl"),
            format!("{gate_in}\n{gate_out}\n"),
        )
        .expect("gates");
        let hook = format!(
            "{{\"timestamp\":{},\"event\":\"preToolUse\",\"session\":\"s-1\"}}",
            (NOW - 5) * 1000
        );
        fs::write(
            home.path().join("hooks/hook_events.jsonl"),
            format!("{hook}\n"),
        )
        .expect("hooks");

        let telemetry = write_telemetry(
            home.path(),
            &logs,
            &home.path().join("reports"),
            NOW - 3 * 3600,
            NOW,
        )
        .expect("telemetry");
        let body = fs::read_to_string(telemetry).expect("read telemetry");

        assert!(body.contains("| bash-block: force push | 1 |"));
        assert!(!body.contains("old"));
        assert!(body.contains("Sessions seen: 1"));
    }
}
