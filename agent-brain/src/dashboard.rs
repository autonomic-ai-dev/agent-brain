//! Local HTML value dashboard from operator stats.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::stats::OperatorStats;

pub fn default_dashboard_path(home: &Path) -> PathBuf {
    home.join("metrics/dashboard.html")
}

pub fn write_dashboard_html(home: &Path, stats: &OperatorStats) -> Result<PathBuf> {
    let dir = home.join("metrics");
    fs::create_dir_all(&dir).with_context(|| format!("create {}", dir.display()))?;
    let path = default_dashboard_path(home);
    let html = render_html(stats);
    fs::write(&path, html).with_context(|| format!("write {}", path.display()))?;
    Ok(path)
}

fn render_html(stats: &OperatorStats) -> String {
    let value = &stats.value;
    let period = stats.period_days;
    let route_calls = stats.period.route_calls;
    let avg_routed = stats.period.avg_routed_tokens;
    let cache_pct = stats.period.cache_hit_rate * 100.0;
    let p95 = stats.period.p95_latency_ms;
    let memories = value.memories_committed;
    let blocked = value.blocked_full_reads;
    let tool_calls = stats.period.tool_calls;
    let index = stats.index.total_indexed;
    let active_mem = stats.index.active_memories;
    let must_apply_routes = stats.period.routes_with_constraints;
    let total_saved_tokens = stats.period.total_saved_tokens + stats.period.tool_tokens_saved;
    let recipes_promoted = stats.period.total_must_apply;
    let generated = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>agent-brain — Cognitive Governor Dashboard</title>
<style>
  :root {{
    --bg: #090b10;
    --card: #11141c;
    --card-border: #212636;
    --text: #f0f3f8;
    --muted: #848c9f;
    --accent: #10b981;
    --accent-dim: rgba(16, 185, 129, 0.12);
    --blue: #3b82f6;
    --amber: #f59e0b;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    font-family: ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.5;
    padding: 2.5rem 1.5rem 4rem;
  }}
  .wrap {{ max-width: 980px; margin: 0 auto; }}
  .badge {{
    display: inline-flex;
    align-items: center;
    gap: 0.4rem;
    font-family: ui-monospace, SFMono-Regular, Menlo, monospace;
    font-size: 0.75rem;
    padding: 0.25rem 0.65rem;
    border-radius: 9999px;
    background: rgba(255, 255, 255, 0.04);
    border: 1px solid var(--card-border);
    color: var(--muted);
    margin-bottom: 0.75rem;
  }}
  .dot {{ width: 6px; height: 6px; border-radius: 50%; background: var(--accent); }}
  h1 {{ font-size: 1.65rem; font-weight: 700; margin: 0 0 0.35rem; letter-spacing: -0.02em; }}
  .sub {{ color: var(--muted); font-size: 0.85rem; margin-bottom: 2rem; font-family: ui-monospace, monospace; }}
  .hero {{
    background: linear-gradient(180deg, #131722 0%, #0c0e14 100%);
    border: 1px solid var(--card-border);
    border-radius: 12px;
    padding: 1.75rem 2rem;
    margin-bottom: 1.75rem;
  }}
  .hero .label {{ font-size: 0.8rem; text-transform: uppercase; letter-spacing: 0.05em; color: var(--muted); font-weight: 600; font-family: ui-monospace, monospace; }}
  .hero .big {{ font-size: 2.5rem; font-weight: 800; color: #ffffff; letter-spacing: -0.03em; margin-top: 0.2rem; }}
  .hero .line {{ font-size: 0.95rem; margin-top: 0.4rem; color: var(--muted); }}
  .grid {{
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(220px, 1fr));
    gap: 1rem;
    margin-bottom: 2rem;
  }}
  .card {{
    background: var(--card);
    border: 1px solid var(--card-border);
    border-radius: 10px;
    padding: 1.25rem;
  }}
  .card .label {{ font-size: 0.7rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); font-family: ui-monospace, monospace; font-weight: 600; }}
  .card .num {{ font-size: 1.75rem; font-weight: 700; margin-top: 0.4rem; color: #ffffff; }}
  .card .hint {{ font-size: 0.75rem; color: var(--muted); margin-top: 0.35rem; }}
  .highlight {{ color: var(--accent); }}
  .section-title {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); margin: 0 0 0.85rem; font-family: ui-monospace, monospace; font-weight: 600; }}
  .note-box {{ background: rgba(255, 255, 255, 0.02); border: 1px solid var(--card-border); border-radius: 10px; padding: 1.25rem; font-size: 0.85rem; color: var(--muted); }}
  .foot {{ margin-top: 2.5rem; font-size: 0.75rem; color: var(--muted); font-family: ui-monospace, monospace; border-top: 1px solid var(--card-border); padding-top: 1rem; }}
  .foot code {{ background: var(--card); padding: 0.15rem 0.35rem; border-radius: 4px; }}
</style>
</head>
<body>
<div class="wrap">
  <div class="badge"><span class="dot"></span> Local Cognitive Governor Active</div>
  <h1>agent-brain local metrics</h1>
  <p class="sub">Period: last {period} days · generated {generated} · local-first telemetry</p>

  <div class="hero">
    <div class="label">Total Tokens Saved via Knapsack &amp; Tool Bounding</div>
    <div class="big">{total_saved_tokens} <span style="font-size:1.25rem; font-weight:500; color:var(--accent);">tok</span></div>
    <p class="line">Enforced across {route_calls} turns · Avg context ~{avg_routed:.0} tok · p95 {p95}ms · cache {cache_pct:.1}%</p>
  </div>

  <div class="grid">
    <div class="card">
      <div class="label">Tokens Saved</div>
      <div class="num highlight">{total_saved_tokens}</div>
      <div class="hint">Knapsack packing + bounded reads</div>
    </div>
    <div class="card">
      <div class="label">Dangerous / Unscouted Blocks</div>
      <div class="num" style="color:var(--amber);">{blocked}</div>
      <div class="hint">bash-gate &amp; scoped read fences</div>
    </div>
    <div class="card">
      <div class="label">Recipes Promoted</div>
      <div class="num" style="color:var(--accent);">{recipes_promoted}</div>
      <div class="hint">exit 0 verified to must_apply</div>
    </div>
    <div class="card">
      <div class="label">Route Calls</div>
      <div class="num">{route_calls}</div>
      <div class="hint">p95 {p95}ms · cache {cache_pct:.1}%</div>
    </div>
    <div class="card">
      <div class="label">Supervisor Turns</div>
      <div class="num">{must_apply_routes}</div>
      <div class="hint">Active adherence constraints</div>
    </div>
    <div class="card">
      <div class="label">Bounded Tool Calls</div>
      <div class="num">{tool_calls}</div>
      <div class="hint">grep, head, tail, file_summary</div>
    </div>
    <div class="card">
      <div class="label">Durable Memories</div>
      <div class="num">{memories}</div>
      <div class="hint">{active_mem} active across sessions</div>
    </div>
    <div class="card">
      <div class="label">Indexed Knowledge</div>
      <div class="num">{index}</div>
      <div class="hint">Skills, rules, agents, memories</div>
    </div>
  </div>

  <section>
    <div class="section-title">Cognitive Governor Status</div>
    <div class="note-box">
      These are authentic local operational telemetry points collected on this machine via SQLite WAL and host pre-tool hooks. Refresh anytime via <code>agent-brain dashboard --open</code>.
    </div>
  </section>

  <p class="foot">Local only · <code>~/.agent_brain/metrics/dashboard.html</code> · strictly zero telemetry egress</p>
</div>
</body>
</html>
"#
    )
}
