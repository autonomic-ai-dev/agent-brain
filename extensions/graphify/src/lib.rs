pub mod cli;
pub mod ingest;
pub mod jobs;
pub mod process;
pub mod query;
pub mod repos;
pub mod rpc;
pub mod types;

use std::path::Path;

use tekmor::settings::GraphifySettings;

pub fn settings_or_default(home: &Path) -> GraphifySettings {
    tekmor::settings::AgentBrainSettings::load(home).graphify
}
