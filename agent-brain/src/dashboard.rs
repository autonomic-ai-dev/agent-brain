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
    let generated = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8"/>
<meta name="viewport" content="width=device-width, initial-scale=1"/>
<title>agent-brain — Local metrics</title>
<style>
  :root {{
    --bg: #0f1117;
    --card: #1a1d27;
    --border: #2a2f3d;
    --text: #e8eaef;
    --muted: #8b93a7;
    --accent: #6ee7b7;
    --accent2: #60a5fa;
    --warn: #fbbf24;
  }}
  * {{ box-sizing: border-box; }}
  body {{
    margin: 0;
    font-family: ui-sans-serif, system-ui, -apple-system, sans-serif;
    background: var(--bg);
    color: var(--text);
    line-height: 1.5;
    padding: 2rem 1.5rem 3rem;
  }}
  .wrap {{ max-width: 960px; margin: 0 auto; }}
  h1 {{ font-size: 1.5rem; font-weight: 600; margin: 0 0 0.25rem; }}
  .sub {{ color: var(--muted); font-size: 0.9rem; margin-bottom: 2rem; }}
  .hero {{
    background: linear-gradient(135deg, #1e293b 0%, #0f172a 100%);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 1.75rem 2rem;
    margin-bottom: 1.5rem;
  }}
  .hero .big {{ font-size: 2.25rem; font-weight: 700; color: var(--accent); letter-spacing: -0.02em; }}
  .hero .line {{ font-size: 1.05rem; margin-top: 0.5rem; color: var(--muted); }}
  .grid {{
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 1rem;
    margin-bottom: 1.5rem;
  }}
  .card {{
    background: var(--card);
    border: 1px solid var(--border);
    border-radius: 10px;
    padding: 1.25rem;
  }}
  .card .label {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.06em; color: var(--muted); }}
  .card .num {{ font-size: 1.75rem; font-weight: 600; margin-top: 0.35rem; }}
  .card .hint {{ font-size: 0.8rem; color: var(--muted); margin-top: 0.35rem; }}
  section h2 {{ font-size: 0.85rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); margin: 0 0 0.75rem; }}
  .foot {{ margin-top: 2rem; font-size: 0.8rem; color: var(--muted); }}
  .foot code {{ background: var(--card); padding: 0.15rem 0.4rem; border-radius: 4px; }}
</style>
</head>
<body>
<div class="wrap">
  <h1>agent-brain local metrics</h1>
  <p class="sub">Last {period} days · generated {generated} · live usage only (not CI fixture proofs)</p>

  <div class="hero">
    <div class="big">{route_calls} routes</div>
    <p class="line">Avg routed context ~{avg_routed:.0} tok · p95 {p95}ms · cache {cache_pct:.0}%</p>
  </div>

  <div class="grid">
    <div class="card">
      <div class="label">Route calls</div>
      <div class="num">{route_calls}</div>
      <div class="hint">p95 {p95}ms · cache {cache_pct:.0}%</div>
    </div>
    <div class="card">
      <div class="label">Memories committed</div>
      <div class="num">{memories}</div>
      <div class="hint">{active_mem} active in index</div>
    </div>
    <div class="card">
      <div class="label">Full-read steers</div>
      <div class="num">{blocked}</div>
      <div class="hint">grep/head instead of cat</div>
    </div>
    <div class="card">
      <div class="label">Token tool calls</div>
      <div class="num">{tool_calls}</div>
      <div class="hint">bounded reads via MCP</div>
    </div>
    <div class="card">
      <div class="label">Supervisor routes</div>
      <div class="num">{must_apply_routes}</div>
      <div class="hint">must_apply enforced</div>
    </div>
    <div class="card">
      <div class="label">Index size</div>
      <div class="num">{index}</div>
      <div class="hint">skills, rules, agents, memory</div>
    </div>
  </div>

  <section>
    <h2>Note</h2>
    <p style="color:var(--muted); font-size:0.95rem;">These are local operational metrics from your machine. CI fixture Recall@3 / dollar ROI are not shown here. Refresh with <code>agent-brain dashboard --open</code>.</p>
  </section>

  <p class="foot">Local only · <code>~/.agent_brain/metrics/dashboard.html</code> · not sent to any server</p>
</div>
</body>
</html>
"#
    )
}
