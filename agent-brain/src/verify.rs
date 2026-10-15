//! Verifier-sourced outcomes: `agent-brain verify -- <cmd>`.
//!
//! An agent with shell access can bypass this path; hashes of verify config /
//! scenario dirs make drift visible in `agent-brain report`.

use std::fs;
use std::path::Path;
use std::process::Command;

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::db::store::BrainStore;

const MIN_SAMPLE_FOR_RATES: usize = 30;

#[derive(Debug, Clone, serde::Serialize)]
pub struct VerifyReport {
    pub id: String,
    pub exit_code: i32,
    pub success: bool,
    pub cause_tag: Option<String>,
    pub verify_config_hash: Option<String>,
    pub scenario_dir_hash: Option<String>,
    pub cmd: String,
}

pub fn run_verify(
    store: &BrainStore,
    cmd: &[String],
    route_log_id: Option<&str>,
    cause_tag: Option<&str>,
    config_path: Option<&Path>,
    scenarios_dir: Option<&Path>,
) -> Result<VerifyReport> {
    if cmd.is_empty() {
        bail!("usage: agent-brain verify [--config PATH] [--scenarios DIR] [--route-log-id ID] [--cause TAG] -- <cmd> [args...]");
    }
    let status = Command::new(&cmd[0])
        .args(&cmd[1..])
        .status()
        .with_context(|| format!("failed to spawn {}", cmd[0]))?;
    let exit_code = status.code().unwrap_or(1);
    let success = exit_code == 0;
    let config_hash = config_path.map(hash_path).transpose()?;
    let scenarios_hash = scenarios_dir.map(hash_tree).transpose()?;
    let cause = cause_tag
        .map(|s| s.to_string())
        .or_else(|| {
            if success {
                None
            } else {
                Some(cmd.join(" "))
            }
        });
    let device_id = store.ensure_device_id()?;
    let id = store.insert_verify_outcome(
        route_log_id,
        exit_code,
        cause.as_deref(),
        config_hash.as_deref(),
        scenarios_hash.as_deref(),
        &cmd.join(" "),
        &device_id,
    )?;
    Ok(VerifyReport {
        id,
        exit_code,
        success,
        cause_tag: cause,
        verify_config_hash: config_hash,
        scenario_dir_hash: scenarios_hash,
        cmd: cmd.join(" "),
    })
}

pub fn print_report(store: &BrainStore) -> Result<()> {
    let rows = store.list_verify_outcomes(500)?;
    let n = rows.len();
    let success = rows.iter().filter(|r| r.exit_code == 0).count();
    let fail = n.saturating_sub(success);
    println!("verify outcomes: n={n} success={success} failure={fail}");
    if n < MIN_SAMPLE_FOR_RATES {
        println!(
            "rates suppressed (need n>={MIN_SAMPLE_FOR_RATES}; showing counts only)"
        );
    } else {
        let rate = success as f64 / n as f64;
        println!("first-pass success rate: {rate:.3}");
    }

    // Hash drift vs most common config hash
    let mut config_counts: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for r in &rows {
        if let Some(h) = &r.verify_config_hash {
            *config_counts.entry(h.clone()).or_default() += 1;
        }
    }
    if let Some((baseline, _)) = config_counts.iter().max_by_key(|(_, c)| *c) {
        let drifted: Vec<_> = rows
            .iter()
            .filter(|r| {
                r.verify_config_hash
                    .as_ref()
                    .is_some_and(|h| h != baseline)
            })
            .collect();
        if !drifted.is_empty() {
            println!(
                "WARNING: {} runs used a different verify_config_hash than the modal baseline",
                drifted.len()
            );
        }
    }

    // Note: agent store_trajectory claims are not included here.
    println!("note: only agent-brain verify -- outcomes are authoritative; shell bypass is possible");
    Ok(())
}

fn hash_path(path: &Path) -> Result<String> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    Ok(format!("{:x}", Sha256::digest(&bytes)))
}

fn hash_tree(dir: &Path) -> Result<String> {
    let mut hasher = Sha256::new();
    let mut paths: Vec<_> = WalkDir::new(dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
        .map(|e| e.into_path())
        .collect();
    paths.sort();
    for path in paths {
        let rel = path.strip_prefix(dir).unwrap_or(&path);
        hasher.update(rel.to_string_lossy().as_bytes());
        hasher.update(fs::read(&path)?);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
