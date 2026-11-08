use std::path::Path;
use std::process::{Command, Output};

use agent_brain::db::store::BrainStore;
use agent_brain::stats::{agstack_usage, skill_mix};

fn route_items(items: &[(&str, &str)]) -> String {
    let rows: Vec<serde_json::Value> = items
        .iter()
        .map(|(kind, topic)| serde_json::json!({ "id": topic, "type": kind, "topic": topic, "score": 1.0 }))
        .collect();
    serde_json::to_string(&rows).unwrap()
}

#[test]
fn a_route_that_surfaces_the_skill_is_counted_once() {
    let usage = agstack_usage(&[route_items(&[
        ("skill", "agstack-mode"),
        ("skill", "verification"),
    ])]);
    assert_eq!(usage.routes, 1);
    assert_eq!(usage.routes_with_agstack_skill, 1);
    assert_eq!(usage.by_skill.get("agstack-mode"), Some(&1));
    assert_eq!(usage.by_skill.get("verification"), Some(&1));
}

#[test]
fn routes_without_the_skill_are_counted_but_not_surfaced() {
    let usage = agstack_usage(&[route_items(&[("skill", "rust-errors")]), route_items(&[])]);
    assert_eq!(usage.routes, 2);
    assert_eq!(usage.routes_with_agstack_skill, 0);
    assert!(usage.by_skill.is_empty());
    assert_eq!(skill_mix(&usage), "none");
}

#[test]
fn only_skill_items_count_so_a_memory_named_verification_does_not() {
    let usage = agstack_usage(&[route_items(&[("memory", "verification")])]);
    assert_eq!(usage.routes_with_agstack_skill, 0);
}

#[test]
fn unreadable_item_log_still_counts_the_route() {
    let usage = agstack_usage(&["not json".to_string()]);
    assert_eq!(usage.routes, 1);
    assert_eq!(usage.routes_with_agstack_skill, 0);
}

#[test]
fn skill_mix_lists_each_skill_with_its_count() {
    let usage = agstack_usage(&[
        route_items(&[("skill", "agstack-mode")]),
        route_items(&[("skill", "agstack-mode"), ("skill", "verification")]),
    ]);
    assert_eq!(skill_mix(&usage), "agstack-mode 2, verification 1");
}

fn stats_markdown(home: &Path, cwd: &Path) -> Output {
    Command::new(env!("CARGO_BIN_EXE_agent-brain"))
        .args(["stats", "--markdown"])
        .current_dir(cwd)
        .env("AGENT_BRAIN_HOME", home)
        .output()
        .expect("run agent-brain stats")
}

#[test]
fn stats_reports_a_real_route_that_surfaced_agstack_mode() {
    let home = tempfile::tempdir().expect("home");
    let cwd = tempfile::tempdir().expect("cwd");
    let first = stats_markdown(home.path(), cwd.path());
    assert!(first.status.success());
    assert!(String::from_utf8_lossy(&first.stdout)
        .contains("| agstack skills surfaced in routes | 0 of 0 (none) |"));

    let store = BrainStore::open(&home.path().join("data/brain.db")).expect("open brain database");
    store
        .insert_retrieval_log(
            "route-agstack-1",
            "query-hash",
            "route",
            &route_items(&[("skill", "agstack-mode")]),
            120,
            false,
            false,
            40,
            None,
            None,
            Some(0),
        )
        .expect("insert route");
    drop(store);

    let second = stats_markdown(home.path(), cwd.path());
    let text = String::from_utf8_lossy(&second.stdout).into_owned();
    assert!(
        text.contains("| agstack skills surfaced in routes | 1 of 1 (agstack-mode 1) |"),
        "{text}"
    );
}
