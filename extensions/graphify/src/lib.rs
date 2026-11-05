pub mod cli;
pub mod ingest;
pub mod jobs;
pub mod process;
pub mod query;
pub mod repos;
pub mod rpc;
pub mod types;

use std::path::Path;

use agent_brain::settings::GraphifySettings;

pub fn settings_or_default(home: &Path) -> GraphifySettings {
    agent_brain::settings::AgentBrainSettings::load(home).graphify
}
