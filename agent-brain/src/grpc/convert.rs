use crate::grpc::pb::{
    AgentRec as PbAgentRec, ContextBundle as PbContextBundle, MemoryRec as PbMemoryRec,
    MustApply as PbMustApply, RouteLimits as PbRouteLimits, RouteTaskRequest, RouteTaskResponse,
    RouteWarning as PbRouteWarning, RuleRec as PbRuleRec, SkillRec as PbSkillRec, TaskKind as PbTaskKind,
};
use crate::types::{
    AgentRec, ContextBundle, MemoryRec, MustApply, RouteLimits, RouteTaskResponse as RustRouteResponse,
    RouteWarning, RuleRec, SkillRec, TaskKind,
};

pub fn task_kind_to_proto(kind: TaskKind) -> i32 {
    match kind {
        TaskKind::Implementing => PbTaskKind::Implementing as i32,
        TaskKind::Verification => PbTaskKind::Verification as i32,
        TaskKind::Debugging => PbTaskKind::Debugging as i32,
        TaskKind::Review => PbTaskKind::Review as i32,
        TaskKind::Architecture => PbTaskKind::Architecture as i32,
    }
}

pub fn task_kind_from_proto(value: i32) -> Option<TaskKind> {
    match PbTaskKind::try_from(value).ok()? {
        PbTaskKind::Implementing => Some(TaskKind::Implementing),
        PbTaskKind::Verification => Some(TaskKind::Verification),
        PbTaskKind::Debugging => Some(TaskKind::Debugging),
        PbTaskKind::Review => Some(TaskKind::Review),
        PbTaskKind::Architecture => Some(TaskKind::Architecture),
        PbTaskKind::Unspecified => None,
    }
}

pub fn limits_from_proto(limits: Option<PbRouteLimits>) -> RouteLimits {
    let limits = limits.unwrap_or(PbRouteLimits {
        agents: 0,
        skills: 0,
        rules: 0,
        memory: 0,
    });
    RouteLimits {
        agents: limits.agents as usize,
        skills: limits.skills as usize,
        rules: limits.rules as usize,
        memory: limits.memory as usize,
    }
    .normalize()
}

pub fn route_request_from_proto(req: RouteTaskRequest) -> crate::bridge::BridgeRouteRequest {
    crate::bridge::BridgeRouteRequest {
        user_message: req.user_message,
        cwd: req.current_working_directory,
        open_files: req.open_files,
        max_tokens: if req.max_tokens == 0 {
            500
        } else {
            req.max_tokens as usize
        },
        limits: limits_from_proto(req.limits),
        phase: req.phase,
        task_kind: task_kind_from_proto(req.task_kind).map(|k| k.as_str().to_string()),
    }
}

pub fn route_response_to_proto(resp: RustRouteResponse) -> RouteTaskResponse {
    let task_kind = resp
        .task_kind
        .as_deref()
        .and_then(TaskKind::parse)
        .map(task_kind_to_proto)
        .unwrap_or(PbTaskKind::Unspecified as i32);
    RouteTaskResponse {
        recommended_agents: resp.recommended_agents.into_iter().map(agent_to_proto).collect(),
        recommended_skills: resp.recommended_skills.into_iter().map(skill_to_proto).collect(),
        applicable_rules: resp.applicable_rules.into_iter().map(rule_to_proto).collect(),
        relevant_memory: resp.relevant_memory.into_iter().map(memory_to_proto).collect(),
        must_apply: resp.must_apply.into_iter().map(must_apply_to_proto).collect(),
        warnings: resp.warnings.into_iter().map(warning_to_proto).collect(),
        recommended_phase: resp.recommended_phase,
        tokens_used: resp.tokens_used as u32,
        tokens_budget: resp.tokens_budget as u32,
        cache_hit: resp.cache_hit,
        latency_ms: resp.latency_ms,
        log_id: resp.log_id,
        index_total: resp.index_total as u32,
        briefing: resp.briefing,
        task_kind,
        route_confidence: resp.route_confidence,
        escalate_recommended: resp.escalate_recommended,
        context_bundle: resp.context_bundle.map(bundle_to_proto),
    }
}

fn agent_to_proto(rec: AgentRec) -> PbAgentRec {
    PbAgentRec {
        name: rec.name,
        path: rec.path,
        rationale: rec.rationale,
        score: rec.score,
    }
}

fn skill_to_proto(rec: SkillRec) -> PbSkillRec {
    PbSkillRec {
        name: rec.name,
        path: rec.path,
        rationale: rec.rationale,
        score: rec.score,
    }
}

fn rule_to_proto(rec: RuleRec) -> PbRuleRec {
    PbRuleRec {
        topic: rec.topic,
        text: rec.text,
        source_path: rec.source_path,
        score: rec.score,
    }
}

fn memory_to_proto(rec: MemoryRec) -> PbMemoryRec {
    PbMemoryRec {
        topic: rec.topic,
        text: rec.text,
        score: rec.score,
    }
}

fn must_apply_to_proto(rec: MustApply) -> PbMustApply {
    PbMustApply {
        topic: rec.topic,
        text: rec.text,
    }
}

fn warning_to_proto(rec: RouteWarning) -> PbRouteWarning {
    PbRouteWarning {
        topic: rec.topic,
        message: rec.message,
    }
}

fn bundle_to_proto(bundle: ContextBundle) -> PbContextBundle {
    PbContextBundle {
        team_rules: bundle.team_rules.into_iter().map(rule_to_proto).collect(),
        negative_memory: bundle.negative_memory.into_iter().map(memory_to_proto).collect(),
        skill_docs: bundle.skill_docs.into_iter().map(skill_to_proto).collect(),
        agents: bundle.agents.into_iter().map(agent_to_proto).collect(),
        observations: bundle.observations.into_iter().map(memory_to_proto).collect(),
    }
}
