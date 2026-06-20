use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowDef {
    pub name: String,
    pub start_node: String,
    pub nodes: Vec<WorkflowNodeDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNodeDef {
    pub name: String,
    pub kind: String,
    pub description: Option<String>,
}

pub fn discover_workflows(workflow_dirs: &[PathBuf]) -> Vec<(PathBuf, WorkflowDef)> {
    let mut results = Vec::new();
    for dir in workflow_dirs {
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(dir).max_depth(2).into_iter().filter_map(|e| e.ok()) {
            if entry.file_type().is_file() {
                let path = entry.path();
                if path.extension().map(|e| e == "yaml").unwrap_or(false) {
                    if let Ok(wf) = parse_workflow(path) {
                        results.push((path.to_path_buf(), wf));
                    }
                }
            }
        }
    }
    results
}

pub fn parse_workflow(path: &Path) -> anyhow::Result<WorkflowDef> {
    let content = fs::read_to_string(path)?;
    let wf: WorkflowDef = serde_yaml::from_str(&content)?;
    Ok(wf)
}
