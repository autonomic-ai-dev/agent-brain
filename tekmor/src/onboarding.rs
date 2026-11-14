//! Post-install onboarding copy — USP tagline and getting-started checklist.

use std::path::Path;

pub const USP_TAGLINE: &str =
    "Route ~500 tokens of the right skills from thousands — hooks make it mandatory.";

pub const USP_SUBLINE: &str =
    "Loads a small routed set of skills/rules/memory each turn (see: tekmor briefing).";

pub fn print_onboarding(home: &Path, has_briefing: bool) {
    println!();
    println!("tekmor — {USP_TAGLINE}");
    println!("{USP_SUBLINE}");
    println!();
    println!("Get value in 5 minutes:");
    println!("  1. Restart Cursor · Settings → MCP → enable tekmor");
    println!("  2. tekmor add @agstack          # universal engineering operating system (default on doctor --fix)");
    println!("     tekmor add @supervisor       # token-efficient ops + execution supervisor");
    println!("     MCP tools: grep_search, file_summary, read_file_head, read_file_tail");
    println!("     (or @starter / @nextjs / owner/repo)");
    println!("  3. Open Agent mode · send any task   # hooks require route_task first");
    println!("  4. tekmor briefing              # skills/rules routed this turn");
    println!("  5. tekmor stats                 # local route/latency metrics");
    println!("  6. tekmor doctor --fix          # if MCP or hooks look wrong");
    println!();
    if has_briefing {
        println!("You already have a route log → tekmor briefing");
    } else {
        println!("No route yet → run one agent turn, then: tekmor briefing");
    }
    println!();
    println!("Curated packs:  tekmor registry list");
    println!("Team setup:       docs/TEAM-WORKFLOW.md (github.com/autonomic-ai-dev/tekmor)");
    println!("Dev proofs: tekmor proofs --ci");
    if let Ok(config) = crate::config::Config::load() {
        if let Ok(store) = crate::db::store::BrainStore::open(&config.db_path) {
            if let Ok(stats) = crate::stats::collect(&store, &config, 7) {
                println!(
                    "Metrics (7d):     {}",
                    crate::stats::format_summary_line(&stats)
                );
            }
        }
    }
    let _ = home;
}

pub fn print_install_success(with_starter: bool) {
    println!();
    println!("✓ tekmor installed");
    println!("  {USP_TAGLINE}");
    if with_starter {
        println!("  Starter pack (@starter) requested — check output above");
    } else {
        println!("  Next: tekmor add @starter");
    }
    println!("  Then: restart Cursor · enable MCP · tekmor onboarding");
}
