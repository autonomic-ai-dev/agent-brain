//! Post-install onboarding copy — USP tagline and getting-started checklist.

use std::path::Path;

pub const USP_TAGLINE: &str =
    "Route ~500 tokens of the right skills from thousands — hooks make it mandatory.";

pub const USP_SUBLINE: &str =
    "Cut input context ~90–99% vs loading your full skill library (see: agent-brain briefing).";

pub fn print_onboarding(home: &Path, has_briefing: bool) {
    println!();
    println!("agent-brain — {USP_TAGLINE}");
    println!("{USP_SUBLINE}");
    println!();
    println!("Get value in 5 minutes:");
    println!("  1. Restart Cursor · Settings → MCP → enable agent-brain");
    println!("  2. agent-brain add @supervisor       # token-efficient ops + execution supervisor");
    println!("     MCP tools: grep_search, file_summary, read_file_head, read_file_tail");
    println!("     (or @starter / @nextjs / owner/repo)");
    println!("  3. Open Agent mode · send any task   # hooks require route_task first");
    println!("  4. agent-brain briefing              # skills routed + token savings");
    println!("  5. agent-brain stats                 # index + savings + adoption milestones");
    println!("  6. agent-brain doctor --fix          # if MCP or hooks look wrong");
    println!();
    if has_briefing {
        println!("You already have a route log → agent-brain briefing");
    } else {
        println!("No route yet → run one agent turn, then: agent-brain briefing");
    }
    println!();
    println!("Curated packs:  agent-brain registry list");
    println!("Team setup:       docs/TEAM-WORKFLOW.md (github.com/aeswibon/agent-brain)");
    println!("Proof/benchmarks: docs/benchmarks/ (Recall@3 on 2000-skill index)");
    if let Ok(config) = crate::config::Config::load() {
        if let Ok(store) = crate::db::store::BrainStore::open(&config.db_path) {
            if let Ok(stats) = crate::stats::collect(&store, &config, 7) {
                println!("Metrics (7d):     {}", crate::stats::format_summary_line(&stats));
            }
        }
    }
    let _ = home;
}

pub fn print_install_success(with_starter: bool) {
    println!();
    println!("✓ agent-brain installed");
    println!("  {USP_TAGLINE}");
    if with_starter {
        println!("  Starter pack (@starter) requested — check output above");
    } else {
        println!("  Next: agent-brain add @starter");
    }
    println!("  Then: restart Cursor · enable MCP · agent-brain onboarding");
}
