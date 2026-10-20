//! v0.23 orchestrator bridge — task_kind policies, confidence, and context bundles.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::types::{
    ContextBundle, MemoryRec, RouteLimits, RouteTaskResponse, ScoredItem, SkillRec, TaskKind,
};

const ESCALATE_CONFIDENCE: f64 = 0.45;

pub fn resolve_task_kind(explicit: Option<&str>, user_message: &str) -> TaskKind {
    explicit
        .and_then(TaskKind::parse)
        .unwrap_or_else(|| crate::workspace::infer_task_kind(user_message))
}

pub fn limits_for_task_kind(kind: TaskKind, base: RouteLimits) -> RouteLimits {
    let base = base.normalize();
    match kind {
        TaskKind::Verification => RouteLimits {
            agents: base.agents.min(1),
            skills: base.skills.min(2),
            rules: base.rules.max(3).min(5),
            memory: base.memory.min(3),
        },
        TaskKind::Architecture => RouteLimits {
            agents: base.agents.max(2).min(4),
            skills: base.skills.max(2).min(5),
            rules: base.rules.max(2).min(5),
            memory: base.memory.min(3),
        },
        TaskKind::Review => RouteLimits {
            agents: base.agents.min(2),
            skills: base.skills.min(2),
            rules: base.rules.max(2).min(5),
            memory: base.memory.min(4),
        },
        TaskKind::Debugging => RouteLimits {
            agents: base.agents.min(2),
            skills: base.skills.min(3),
            rules: base.rules.min(3),
            memory: base.memory.max(3).min(6),
        },
        TaskKind::Implementing => base,
    }
}

pub fn enrich_route_response(
    resp: &mut RouteTaskResponse,
    scored: &[ScoredItem],
    task_kind: TaskKind,
    _user_message: &str,
) {
    resp.task_kind = Some(task_kind.as_str().to_string());
    resp.route_confidence = compute_route_confidence(resp, scored);
    hydrate_high_confidence_skills(resp);
    resp.context_bundle = Some(build_context_bundle(resp, scored));
    resp.escalate_recommended = should_escalate(resp, task_kind);

    // Fallback: If vector search did not surface a specific workflow, select default based on task_kind
    if resp.suggested_workflow.is_none() {
        let default_wf = match task_kind {
            TaskKind::Verification | TaskKind::Implementing | TaskKind::Debugging => "engineering-safety",
            TaskKind::Review => "deep-code-review",
            TaskKind::Architecture => "system-architect",
        };
        resp.suggested_workflow = Some(default_wf.to_string());
        if resp.must_apply.len() < 3 {
            resp.must_apply.push(crate::types::MustApply {
                topic: "spine-workflow".into(),
                text: format!("Recommended Spine workflow: `{default_wf}`. Orchestrate via `spine_submit_workflow` (workflow=\"{default_wf}\") or `agent-spine run --workflow {default_wf}`."),
            });
        }
    }
}

const SKILL_AUTOLOAD_MIN_SCORE: f64 = 0.8;
const SKILL_AUTOLOAD_MAX_CHARS: usize = 4000;

fn hydrate_high_confidence_skills(resp: &mut RouteTaskResponse) {
    for skill in &mut resp.recommended_skills {
        if skill.score < SKILL_AUTOLOAD_MIN_SCORE {
            continue;
        }
        if skill.text.is_some() {
            continue;
        }
        let path = std::path::Path::new(&skill.path);
        if !path.is_file() {
            continue;
        }
        if let Ok(raw) = std::fs::read_to_string(path) {
            skill.text = Some(raw.chars().take(SKILL_AUTOLOAD_MAX_CHARS).collect());
        }
    }
}

fn compute_route_confidence(resp: &RouteTaskResponse, scored: &[ScoredItem]) -> f64 {
    let top_non_memory = scored
        .iter()
        .filter(|i| !matches!(i.item_type, crate::types::ItemType::Memory))
        .map(|i| i.score)
        .fold(0.0_f64, f64::max);
    let top_memory = scored
        .iter()
        .filter(|i| matches!(i.item_type, crate::types::ItemType::Memory))
        .map(|i| i.score)
        .fold(0.0_f64, f64::max);
    let signal = top_non_memory.max(top_memory * 0.9);
    let coverage = [
        !resp.recommended_agents.is_empty(),
        !resp.recommended_skills.is_empty(),
        !resp.applicable_rules.is_empty(),
        !resp.relevant_memory.is_empty(),
    ]
    .into_iter()
    .filter(|v| *v)
    .count() as f64
        / 4.0;
    let must_apply_boost = if resp.must_apply.is_empty() {
        0.0
    } else {
        0.12
    };
    ((signal * 0.72) + (coverage * 0.28) + must_apply_boost).clamp(0.0, 1.0)
}

fn should_escalate(resp: &RouteTaskResponse, task_kind: TaskKind) -> bool {
    if resp.route_confidence < ESCALATE_CONFIDENCE {
        return true;
    }
    if task_kind == TaskKind::Verification
        && resp.relevant_memory.is_empty()
        && resp.applicable_rules.is_empty()
    {
        return true;
    }
    false
}

pub fn build_context_bundle(resp: &RouteTaskResponse, scored: &[ScoredItem]) -> ContextBundle {
    let negative_memory = negative_memories(resp, scored);
    let observations = observation_memories(resp, scored);
    ContextBundle {
        team_rules: resp.applicable_rules.clone(),
        negative_memory,
        skill_docs: resp
            .recommended_skills
            .iter()
            .map(|skill| SkillRec {
                text: None,
                ..skill.clone()
            })
            .collect(),
        agents: resp.recommended_agents.clone(),
        observations,
    }
}

fn negative_memories(resp: &RouteTaskResponse, scored: &[ScoredItem]) -> Vec<MemoryRec> {
    use crate::workspace::is_low_signal_memory;
    let mut out: Vec<MemoryRec> = resp
        .relevant_memory
        .iter()
        .filter(|m| is_negative_topic_or_text(&m.topic, &m.text))
        .filter(|m| !is_low_signal_memory(&m.topic, None))
        .cloned()
        .collect();
    for item in scored {
        if !matches!(item.item_type, crate::types::ItemType::Memory) {
            continue;
        }
        if is_low_signal_memory(&item.topic, None) {
            continue;
        }
        if item.polarity.as_deref() != Some("negative")
            && !is_negative_topic_or_text(&item.topic, &item.text)
        {
            continue;
        }
        // Same soft gates as recommendations: need lexical affinity.
        if item.lexical < 0.15 || item.score < 0.35 {
            continue;
        }
        let rec = MemoryRec {
            topic: item.topic.clone(),
            text: item.text.chars().take(300).collect(),
            score: item.score,
        };
        if !out.iter().any(|m| m.topic == rec.topic) {
            out.push(rec);
        }
    }
    out
}

fn observation_memories(_resp: &RouteTaskResponse, _scored: &[ScoredItem]) -> Vec<MemoryRec> {
    // Observations stay in inspect/DB — do not re-inject obs/* into every MCP turn.
    Vec::new()
}

fn is_negative_topic_or_text(topic: &str, text: &str) -> bool {
    let lower = text.to_lowercase();
    topic.starts_with("anti-")
        || lower.contains("do not")
        || lower.contains("never ")
        || lower.contains("avoid ")
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BridgeRouteRequest {
    pub user_message: String,
    pub cwd: Option<String>,
    pub open_files: Vec<String>,
    pub max_tokens: usize,
    pub limits: RouteLimits,
    pub phase: Option<String>,
    pub task_kind: Option<String>,
}

#[cfg(test)]
mod tests {
    use crate::types::{AgentRec, RuleRec, SkillRec};
    use super::*;
    use crate::types::{ItemType, MustApply};

    #[test]
    fn verification_tightens_limits() {
        let base = RouteLimits {
            agents: 2,
            skills: 3,
            rules: 2,
            memory: 5,
        };
        let tight = limits_for_task_kind(TaskKind::Verification, base);
        assert_eq!(tight.memory, 3);
        assert!(tight.rules >= 3);
    }

    #[test]
    fn low_confidence_escalates() {
        let mut resp = RouteTaskResponse {
            route_confidence: 0.2,
            task_kind: Some("verification".into()),
            ..Default::default()
        };
        assert!(should_escalate(&resp, TaskKind::Verification));
        resp.route_confidence = 0.9;
        resp.relevant_memory.push(MemoryRec {
            topic: "x".into(),
            text: "y".into(),
            score: 0.8,
        });
        assert!(!should_escalate(&resp, TaskKind::Verification));
    }

    #[test]
    fn context_bundle_partitions_observations() {
        let resp = RouteTaskResponse {
            recommended_agents: vec![AgentRec {
                name: "reviewer".into(),
                path: "/a.md".into(),
                rationale: "r".into(),
                score: 0.8,
            }],
            relevant_memory: vec![
                MemoryRec {
                    topic: "obs/testing".into(),
                    text: "use vitest".into(),
                    score: 0.7,
                },
                MemoryRec {
                    topic: "anti-read".into(),
                    text: "never read dist".into(),
                    score: 0.6,
                },
            ],
            applicable_rules: vec![RuleRec {
                topic: "lint".into(),
                text: "run clippy".into(),
                source_path: "/r.md".into(),
                score: 0.75,
            }],
            must_apply: vec![MustApply {
                topic: "lint".into(),
                text: "run clippy".into(),
            }],
            ..Default::default()
        };
        let bundle = build_context_bundle(&resp, &[]);
        assert_eq!(bundle.agents.len(), 1);
        assert!(bundle.observations.is_empty(), "obs must not re-enter MCP bundle");
        assert_eq!(bundle.negative_memory.len(), 1);
        assert_eq!(bundle.team_rules.len(), 1);
    }

    #[test]
    fn context_bundle_does_not_repeat_skill_text() {
        let resp = RouteTaskResponse {
            recommended_skills: vec![SkillRec {
                name: "testing".into(),
                path: "/s.md".into(),
                rationale: "r".into(),
                score: 0.9,
                text: Some("full skill body".into()),
            }],
            ..Default::default()
        };
        let bundle = build_context_bundle(&resp, &[]);
        assert_eq!(bundle.skill_docs.len(), 1);
        assert_eq!(bundle.skill_docs[0].name, "testing");
        assert!(bundle.skill_docs[0].text.is_none());
        assert_eq!(resp.recommended_skills[0].text.as_deref(), Some("full skill body"));
    }

    #[test]
    fn confidence_uses_scored_signal() {
        let resp = RouteTaskResponse {
            recommended_skills: vec![SkillRec {
                name: "rust".into(),
                path: "/s.md".into(),
                rationale: "r".into(),
                score: 0.9,
                text: None,
            }],
            ..Default::default()
        };
        let scored = vec![ScoredItem {
            id: "1".into(),
            item_type: ItemType::Skill,
            topic: "rust".into(),
            text: "patterns".into(),
            source_path: None,
            scope: "project".into(),
            score: 0.9,
            polarity: None,
            apply_when_matched: false,
            lexical: 0.0,
            scope_key: None,
        }];
        let c = compute_route_confidence(&resp, &scored);
        assert!(c > 0.5);
    }

    #[test]
    fn test_workflow_fallback_by_task_kind() {
        let mut resp = RouteTaskResponse::default();
        enrich_route_response(&mut resp, &[], TaskKind::Verification, "verify clippy");
        assert_eq!(resp.suggested_workflow.as_deref(), Some("engineering-safety"));

        let mut resp_rev = RouteTaskResponse::default();
        enrich_route_response(&mut resp_rev, &[], TaskKind::Review, "review PR");
        assert_eq!(resp_rev.suggested_workflow.as_deref(), Some("deep-code-review"));
    }
}
