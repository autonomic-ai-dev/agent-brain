//! Local HTML value dashboard from operator stats.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::db::store::{DailyUsage, VerifyCommand};
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
    let period = stats.period_days;
    let p = &stats.period;
    let daily = &stats.daily;
    let routed_sum = daily_sum(daily, |d| d.routed_tokens);
    let bounded_sum = daily_sum(daily, |d| d.bounded_read_tokens_saved);
    let verify_passed = daily_sum(daily, |d| d.verify_passed);
    let verify_failed = daily_sum(daily, |d| d.verify_failed);
    let daily_routes = daily_sum(daily, |d| d.route_calls);
    let cache_hits = daily_sum(daily, |d| d.cache_hits);
    let constrained = daily_sum(daily, |d| d.constrained_routes);
    let cache_pct = pct(cache_hits, daily_routes);
    let constrained_pct = pct(constrained, daily_routes);
    let pass_rate = pct(verify_passed, verify_passed + verify_failed);
    let peak_text = daily
        .iter()
        .max_by_key(|d| d.routed_tokens)
        .map_or_else(String::new, |d| {
            format!(" · peak {} tok on {}", d.routed_tokens, d.day)
        });
    let index_mix = stats
        .index
        .by_type
        .iter()
        .map(|(item_type, count)| format!("{item_type} {count}"))
        .collect::<Vec<_>>()
        .join(" · ");
    let total_saved_tokens = p.total_saved_tokens + p.tool_tokens_saved;
    let generated = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC");
    let charts = render_charts(stats);
    let agstack_routes = stats.agstack.routes_with_agstack_skill;
    let agstack_total = stats.agstack.routes;
    let agstack_skills = crate::stats::skill_mix(&stats.agstack);
    let memories = stats.value.memories_committed;
    let blocked = stats.value.blocked_full_reads;
    let active_mem = stats.index.active_memories;
    let index = stats.index.total_indexed;

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
  .wrap {{ max-width: 1040px; margin: 0 auto; }}
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
  .tile summary {{ list-style: none; cursor: pointer; }}
  .tile summary::-webkit-details-marker {{ display: none; }}
  .tile summary::after {{ content: "details"; display: block; margin-top: 0.5rem; font-size: 0.65rem; font-family: ui-monospace, monospace; color: var(--blue); }}
  .tile[open] summary::after {{ content: "hide"; }}
  .tile:hover {{ border-color: #2f3750; }}
  .detail {{ font-size: 0.8rem; color: var(--text); border-top: 1px solid var(--card-border); margin: 0.9rem 0 0; padding-top: 0.8rem; line-height: 1.5; }}
  .tip {{ font-size: 0.75rem; color: var(--muted); margin: -1rem 0 1rem; font-family: ui-monospace, monospace; }}
  .highlight {{ color: var(--accent); }}
  .section-title {{ font-size: 0.75rem; text-transform: uppercase; letter-spacing: 0.08em; color: var(--muted); margin: 2rem 0 0.85rem; font-family: ui-monospace, monospace; font-weight: 600; }}
  .charts {{ display: grid; grid-template-columns: 1fr; gap: 1rem; }}
  @media (min-width: 760px) {{ .charts {{ grid-template-columns: 1fr 1fr; }} }}
  .chart {{ background: var(--card); border: 1px solid var(--card-border); border-radius: 10px; padding: 1.1rem 1.25rem; min-width: 0; }}
  .chart h3 {{ font-size: 0.8rem; margin: 0 0 0.6rem; color: var(--text); font-weight: 600; }}
  .chart svg {{ display: block; }}
  .hrow {{ display: grid; grid-template-columns: minmax(90px, 0.9fr) 2fr auto; gap: 0.6rem; align-items: center; font-size: 0.75rem; margin: 0.4rem 0; }}
  .hlabel {{ overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: ui-monospace, monospace; color: var(--muted); }}
  .htrack {{ display: flex; height: 8px; background: rgba(255, 255, 255, 0.05); border-radius: 4px; overflow: hidden; }}
  .hfill {{ display: block; height: 100%; }}
  .hval {{ color: var(--text); font-variant-numeric: tabular-nums; white-space: nowrap; }}
  .hval small {{ color: var(--muted); }}
  .empty {{ font-size: 0.8rem; color: var(--muted); margin: 0.5rem 0; }}
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
    <div class="label">Context tokens returned by route_task</div>
    <div class="big">{routed_sum} <span style="font-size:1.25rem; font-weight:500; color:var(--accent);">tok</span></div>
    <p class="line">Across {route_calls} routed turns · avg {avg_routed:.0} tok per turn{peak_text}</p>
  </div>

  <p class="tip">Click any tile to see what it counts and where the number comes from.</p>
  <div class="grid">
    <details class="card tile">
      <summary>
        <div class="label">Route calls</div>
        <div class="num">{route_calls}</div>
        <div class="hint">avg {avg_latency:.0} ms · p95 {p95} ms</div>
      </summary>
      <p class="detail">Each route_task response logged in the retrieval log for this period, excluding upstream calls. avg is the mean latency; p95 is the 95th percentile. Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Cache hit rate</div>
        <div class="num">{cache_pct:.1}%</div>
        <div class="hint">{cache_hits} of {daily_routes} routes served from cache</div>
      </summary>
      <p class="detail">Share of route responses served from the route cache (cache_hit in the retrieval log). Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Supervisor turns</div>
        <div class="num">{constrained}</div>
        <div class="hint">{constrained_pct:.0}% of routes carried must_apply constraints</div>
      </summary>
      <p class="detail">Routes whose response carried one or more must_apply constraints (must_apply_count above zero in the retrieval log). Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">must_apply items served</div>
        <div class="num" style="color:var(--accent);">{recipes_promoted}</div>
        <div class="hint">Constraints returned by route_task in this period</div>
      </summary>
      <p class="detail">Total must_apply constraints returned across all routes in the period (the sum of must_apply_count). Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Bounded-read saving, upper bound</div>
        <div class="num highlight">{bounded_sum}</div>
        <div class="hint">Bytes of files that matched ÷ 4, minus tokens returned</div>
      </summary>
      <p class="detail">For each directory grep, the bytes of files that produced a match, divided by 4, minus the tokens returned. It assumes the agent would otherwise have read those files in full, so treat it as an upper bound. Source: tool_log.tokens_saved. Estimate.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Bounded tool calls</div>
        <div class="num">{tool_calls}</div>
        <div class="hint">grep, head, tail, file_summary · avg {tool_avg:.0}% saved per call (estimate)</div>
      </summary>
      <p class="detail">Calls to the bounded tools in the period. The average saving uses the same counterfactual as the bounded-read saving above, so it is an estimate.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Verify runs passed / failed</div>
        <div class="num">{verify_passed} / {verify_failed}</div>
        <div class="hint">Pass rate {pass_rate:.1}% · recorded exit codes from agent-brain verify</div>
      </summary>
      <p class="detail">Verify runs recorded in the period. Passed means exit code 0; failed means any other exit code. Only runs executed by agent-brain verify are recorded, and each run keeps the commit and code hash it tested. Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Full-read steers</div>
        <div class="num" style="color:var(--amber);">{blocked}</div>
        <div class="hint">Native full-file reads steered to bounded tools · {tool_calls} bounded calls</div>
      </summary>
      <p class="detail">Native full-file reads that agent-brain steered to a bounded tool in the period (inefficient_read_events). Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Estimated saving vs naive load</div>
        <div class="num" style="color:var(--muted);">{total_saved_tokens}</div>
        <div class="hint">Assumes each indexed item would cost 120 tokens if loaded. An estimate, not a measurement.</div>
      </summary>
      <p class="detail">Modelled, not measured: indexed items × 120 tokens, minus tokens routed, plus the bounded-read estimate. Shown for context only and left out of the weekly summary.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Durable memories</div>
        <div class="num">{memories}</div>
        <div class="hint">{active_mem} active across sessions</div>
      </summary>
      <p class="detail">Facts stored in the period, and the number currently active across sessions. Measured from the local database.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">Indexed knowledge</div>
        <div class="num">{index}</div>
        <div class="hint">{index_mix}</div>
      </summary>
      <p class="detail">Items in the local index by type (skills, rules, agents, memories, workflows), from the installed packages and the project. Measured.</p>
    </details>
    <details class="card tile">
      <summary>
        <div class="label">agstack skills surfaced</div>
        <div class="num">{agstack_routes}</div>
        <div class="hint">of {agstack_total} routes · {agstack_skills}</div>
      </summary>
      <p class="detail">Routes whose top results included the agstack-mode or verification skill, counted from the route log. Measured. It shows what route_task offered. It does not show whether the agent read the skill.</p>
    </details>
  </div>

  {charts}

  <section>
    <div class="section-title">Cognitive Governor Status</div>
    <div class="note-box">
      These are authentic local operational telemetry points collected on this machine via SQLite WAL and host pre-tool hooks. Refresh anytime via <code>agent-brain dashboard --open</code>.
    </div>
  </section>

  <p class="foot">Local only · the dashboard makes no network calls · every figure is read from the local database. Estimates are labelled.</p>
</div>
</body>
</html>
"#,
        route_calls = p.route_calls,
        avg_routed = p.avg_routed_tokens,
        avg_latency = p.avg_latency_ms,
        p95 = p.p95_latency_ms,
        tool_calls = p.tool_calls,
        tool_avg = p.tool_avg_savings_pct,
        recipes_promoted = p.total_must_apply,
    )
}

fn render_charts(stats: &OperatorStats) -> String {
    let daily = &stats.daily;
    let routes_per_day: Vec<(&str, u64)> = daily
        .iter()
        .map(|d| (d.day.as_str(), d.route_calls))
        .collect();
    let cache_per_day: Vec<(&str, f64)> = daily
        .iter()
        .map(|d| (d.day.as_str(), pct(d.cache_hits, d.route_calls)))
        .collect();
    let routed_per_day: Vec<(&str, u64)> = daily
        .iter()
        .map(|d| (d.day.as_str(), d.routed_tokens))
        .collect();
    let bounded_per_day: Vec<(&str, u64)> = daily
        .iter()
        .map(|d| (d.day.as_str(), d.bounded_read_tokens_saved))
        .collect();
    let verify_per_day: Vec<(&str, u64, u64)> = daily
        .iter()
        .map(|d| (d.day.as_str(), d.verify_passed, d.verify_failed))
        .collect();
    let latency: Vec<(String, u64, String)> = stats
        .latency
        .iter()
        .map(|b| (b.label.clone(), b.count, String::new()))
        .collect();
    let phases: Vec<(String, u64, String)> = stats
        .period
        .phases
        .iter()
        .map(|(phase, count)| (phase.clone(), *count as u64, String::new()))
        .collect();
    let tools: Vec<(String, u64, String)> = stats
        .tools
        .iter()
        .map(|t| {
            (
                t.tool_name.clone(),
                t.calls,
                format!(
                    "{} tok returned · ≤{} tok saved",
                    t.tokens_used, t.tokens_saved
                ),
            )
        })
        .collect();
    let types: Vec<(String, u64, String)> = stats
        .index
        .by_type
        .iter()
        .map(|(item_type, count)| (item_type.clone(), *count as u64, String::new()))
        .collect();

    let routing = [
        chart_box(
            "Route calls per day",
            &bar_chart(&routes_per_day, "#a3e635"),
        ),
        chart_box("Cache hit rate per day (%)", &line_chart(&cache_per_day)),
        chart_box(
            "Context tokens returned per day",
            &bar_chart(&routed_per_day, "#a3e635"),
        ),
        chart_box("Route latency distribution", &hbars(&latency, "#60a5fa")),
        chart_box("Route phases", &hbars(&phases, "#a78bfa")),
    ]
    .concat();
    let savings = [
        chart_box(
            "Bounded-read saving per day (upper bound)",
            &bar_chart(&bounded_per_day, "#60a5fa"),
        ),
        chart_box(
            "Verify runs per day: passed (green), failed (red)",
            &verify_chart(&verify_per_day),
        ),
        chart_box(
            "Verify runs by command",
            &stacked_hbars(&stats.verify_commands),
        ),
        chart_box("Bounded tools by calls", &hbars(&tools, "#60a5fa")),
    ]
    .concat();
    let knowledge = chart_box("Indexed items by type", &hbars(&types, "#10b981"));

    format!(
        "<section><div class=\"section-title\">Routing</div><div class=\"charts\">{routing}</div></section>\
         <section><div class=\"section-title\">Savings and verification</div><div class=\"charts\">{savings}</div></section>\
         <section><div class=\"section-title\">Knowledge base</div><div class=\"charts\">{knowledge}</div></section>"
    )
}

fn chart_box(title: &str, body: &str) -> String {
    format!("<div class=\"chart\"><h3>{}</h3>{body}</div>", esc(title))
}

fn daily_sum(daily: &[DailyUsage], value: impl Fn(&DailyUsage) -> u64) -> u64 {
    daily.iter().map(value).sum()
}

fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64 * 100.0
    }
}

fn esc(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn empty_note() -> String {
    "<p class=\"empty\">No data in this period.</p>".to_string()
}

const CHART_WIDTH: f64 = 640.0;
const CHART_HEIGHT: f64 = 150.0;
const CHART_PAD: f64 = 24.0;

fn bar_chart(points: &[(&str, u64)], color: &str) -> String {
    if points.is_empty() {
        return empty_note();
    }
    let max = points.iter().map(|(_, v)| *v).max().unwrap_or(0).max(1) as f64;
    let plot_h = CHART_HEIGHT - 2.0 * CHART_PAD;
    let slot = (CHART_WIDTH - 2.0 * CHART_PAD) / points.len() as f64;
    let bar_w = (slot * 0.6).max(2.0);
    let mut bars = String::new();
    for (i, (day, value)) in points.iter().enumerate() {
        let h = *value as f64 / max * plot_h;
        let x = CHART_PAD + slot * i as f64 + (slot - bar_w) / 2.0;
        let y = CHART_HEIGHT - CHART_PAD - h;
        bars.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{bar_w:.1}\" height=\"{h:.1}\" fill=\"{color}\"><title>{day}: {value}</title></rect>"
        ));
    }
    svg_frame(
        &bars,
        points.first().map(|p| p.0),
        points.last().map(|p| p.0),
    )
}

fn line_chart(points: &[(&str, f64)]) -> String {
    if points.is_empty() {
        return empty_note();
    }
    let plot_h = CHART_HEIGHT - 2.0 * CHART_PAD;
    let plot_w = CHART_WIDTH - 2.0 * CHART_PAD;
    let last = (points.len() - 1).max(1) as f64;
    let mut coords = Vec::new();
    let mut dots = String::new();
    for (i, (day, value)) in points.iter().enumerate() {
        let x = CHART_PAD + plot_w * i as f64 / last;
        let y = CHART_HEIGHT - CHART_PAD - value.clamp(0.0, 100.0) / 100.0 * plot_h;
        coords.push(format!("{x:.1},{y:.1}"));
        dots.push_str(&format!(
            "<circle cx=\"{x:.1}\" cy=\"{y:.1}\" r=\"3\" fill=\"#a78bfa\"><title>{day}: {value:.1}%</title></circle>"
        ));
    }
    let body = format!(
        "<polyline points=\"{}\" fill=\"none\" stroke=\"#a78bfa\" stroke-width=\"2\"/>{dots}",
        coords.join(" ")
    );
    svg_frame(
        &body,
        points.first().map(|p| p.0),
        points.last().map(|p| p.0),
    )
}

fn verify_chart(points: &[(&str, u64, u64)]) -> String {
    if points.is_empty() {
        return empty_note();
    }
    let max = points
        .iter()
        .map(|(_, p, f)| p + f)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    let plot_h = CHART_HEIGHT - 2.0 * CHART_PAD;
    let slot = (CHART_WIDTH - 2.0 * CHART_PAD) / points.len() as f64;
    let bar_w = (slot * 0.6).max(2.0);
    let mut bars = String::new();
    for (i, (day, passed, failed)) in points.iter().enumerate() {
        let x = CHART_PAD + slot * i as f64 + (slot - bar_w) / 2.0;
        let h_pass = *passed as f64 / max * plot_h;
        let h_fail = *failed as f64 / max * plot_h;
        let y_pass = CHART_HEIGHT - CHART_PAD - h_pass;
        let y_fail = y_pass - h_fail;
        bars.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y_pass:.1}\" width=\"{bar_w:.1}\" height=\"{h_pass:.1}\" fill=\"#4ade80\"><title>{day}: {passed} passed</title></rect>"
        ));
        bars.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{y_fail:.1}\" width=\"{bar_w:.1}\" height=\"{h_fail:.1}\" fill=\"#f87171\"><title>{day}: {failed} failed</title></rect>"
        ));
    }
    svg_frame(
        &bars,
        points.first().map(|p| p.0),
        points.last().map(|p| p.0),
    )
}

fn svg_frame(body: &str, first: Option<&str>, last: Option<&str>) -> String {
    let base_y = CHART_HEIGHT - CHART_PAD + 14.0;
    let first_label = esc(first.unwrap_or_default());
    let last_label = esc(last.unwrap_or_default());
    format!(
        "<svg viewBox=\"0 0 {CHART_WIDTH} {CHART_HEIGHT}\" width=\"100%\" role=\"img\">\
         <line x1=\"{CHART_PAD}\" y1=\"{axis}\" x2=\"{right}\" y2=\"{axis}\" stroke=\"#3f3f46\"/>\
         {body}\
         <text x=\"{CHART_PAD}\" y=\"{base_y}\" font-size=\"11\" fill=\"#a1a1aa\">{first_label}</text>\
         <text x=\"{right}\" y=\"{base_y}\" font-size=\"11\" fill=\"#a1a1aa\" text-anchor=\"end\">{last_label}</text>\
         </svg>",
        axis = CHART_HEIGHT - CHART_PAD,
        right = CHART_WIDTH - CHART_PAD,
    )
}

fn hbars(rows: &[(String, u64, String)], color: &str) -> String {
    if rows.is_empty() {
        return empty_note();
    }
    let max = rows.iter().map(|r| r.1).max().unwrap_or(0).max(1) as f64;
    rows.iter()
        .map(|(label, value, note)| {
            let w = *value as f64 / max * 100.0;
            format!(
                "<div class=\"hrow\"><span class=\"hlabel\" title=\"{l}\">{l}</span><span class=\"htrack\"><span class=\"hfill\" style=\"width:{w:.1}%;background:{color}\"></span></span><span class=\"hval\">{value}<small> {n}</small></span></div>",
                l = esc(label),
                n = esc(note),
            )
        })
        .collect()
}

fn stacked_hbars(rows: &[VerifyCommand]) -> String {
    if rows.is_empty() {
        return empty_note();
    }
    let max = rows
        .iter()
        .map(|r| r.passed + r.failed)
        .max()
        .unwrap_or(0)
        .max(1) as f64;
    rows.iter()
        .map(|r| {
            let pass_w = r.passed as f64 / max * 100.0;
            let fail_w = r.failed as f64 / max * 100.0;
            format!(
                "<div class=\"hrow\"><span class=\"hlabel\" title=\"{c}\">{c}</span><span class=\"htrack\"><span class=\"hfill\" style=\"width:{pass_w:.1}%;background:#4ade80\"></span><span class=\"hfill\" style=\"width:{fail_w:.1}%;background:#f87171\"></span></span><span class=\"hval\">{p} passed <small>{f} failed</small></span></div>",
                c = esc(&r.cmd),
                p = r.passed,
                f = r.failed,
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tallest_bar_height(svg: &str) -> f64 {
        svg.split("height=\"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next()?.parse::<f64>().ok())
            .fold(0.0, f64::max)
    }

    #[test]
    fn empty_period_says_so_instead_of_drawing_bars() {
        let html = bar_chart(&[], "#000");
        assert!(html.contains("No data in this period"));
        assert!(!html.contains("<svg"));
    }

    #[test]
    fn largest_day_fills_the_plot_height() {
        let svg = bar_chart(&[("2026-11-01", 100), ("2026-11-02", 400)], "#000");
        let full = CHART_HEIGHT - 2.0 * CHART_PAD;
        assert!((tallest_bar_height(&svg) - full).abs() < 0.11);
    }

    #[test]
    fn zero_values_draw_zero_height_bars() {
        let svg = bar_chart(&[("2026-11-01", 0)], "#000");
        assert_eq!(tallest_bar_height(&svg), 0.0);
    }

    #[test]
    fn labels_from_data_are_escaped() {
        assert_eq!(esc("<b>&\"'"), "&lt;b&gt;&amp;&quot;&#39;");
        let html = hbars(&[("cargo <x>".to_string(), 1, String::new())], "#000");
        assert!(!html.contains("<x>"));
    }

    #[test]
    fn largest_row_fills_the_track_and_others_scale() {
        let html = hbars(
            &[
                ("a".to_string(), 4, String::new()),
                ("b".to_string(), 2, String::new()),
            ],
            "#000",
        );
        assert!(html.contains("width:100.0%"));
        assert!(html.contains("width:50.0%"));
    }

    #[test]
    fn stacked_rows_split_pass_and_fail_by_total() {
        let html = stacked_hbars(&[VerifyCommand {
            cmd: "run".to_string(),
            passed: 3,
            failed: 1,
        }]);
        assert!(html.contains("width:75.0%"));
        assert!(html.contains("width:25.0%"));
    }

    #[test]
    fn line_chart_clamps_percentages_to_the_plot() {
        let svg = line_chart(&[("2026-11-01", 150.0)]);
        assert!(svg.contains(",24.0"));
    }
}
