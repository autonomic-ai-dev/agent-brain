//! OS keychain-backed secret storage (names in brain.db; values never synced).

use std::io::{self, Write};

use anyhow::{Context, Result};
use keyring::Entry;
use serde::{Deserialize, Serialize};

use crate::db::store::BrainStore;

const SERVICE: &str = "tekmor";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretRef {
    pub name: String,
    pub used_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SecretsStatus {
    pub configured: Vec<String>,
    pub missing: Vec<String>,
    pub stored_in: String,
}

pub fn get_secret(name: &str) -> Result<Option<String>> {
    if let Ok(entry) = Entry::new(SERVICE, name) {
        if let Ok(value) = entry.get_password() {
            if !value.is_empty() {
                return Ok(Some(value));
            }
        }
    }
    Ok(std::env::var(name).ok())
}

pub fn set_secret(name: &str, value: &str) -> Result<()> {
    let entry = Entry::new(SERVICE, name).context("open keychain entry")?;
    entry
        .set_password(value)
        .with_context(|| format!("store secret {name} in keychain"))
}

pub fn secrets_status(store: &BrainStore) -> Result<SecretsStatus> {
    let refs = store.list_secret_refs()?;
    let mut configured = Vec::new();
    let mut missing = Vec::new();
    for reference in refs {
        if get_secret(&reference.name)?.is_some() {
            configured.push(reference.name);
        } else {
            missing.push(reference.name);
        }
    }
    configured.sort();
    missing.sort();
    Ok(SecretsStatus {
        configured,
        missing,
        stored_in: "keychain".into(),
    })
}

pub fn setup_missing_interactive(store: &BrainStore) -> Result<()> {
    let status = secrets_status(store)?;
    if status.missing.is_empty() {
        println!("All secret references are configured.");
        return Ok(());
    }

    println!("Missing secrets for upstream MCP:");
    for name in &status.missing {
        let used_by = store
            .list_secret_refs()?
            .into_iter()
            .find(|r| &r.name == name)
            .map(|r| r.used_by)
            .unwrap_or_else(|| "upstream".into());
        print!("Enter value for {name} (used by {used_by}): ");
        io::stdout().flush()?;
        let mut line = String::new();
        io::stdin().read_line(&mut line)?;
        let value = line.trim();
        if value.is_empty() {
            anyhow::bail!("empty value for {name}");
        }
        set_secret(name, value)?;
        println!("Stored {name} in keychain.");
    }
    Ok(())
}

pub fn missing_secret_names(store: &BrainStore) -> Result<Vec<String>> {
    Ok(secrets_status(store)?.missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_fallback_for_secret_lookup() {
        std::env::set_var("TEKMOR_TEST_SECRET", "from-env");
        assert_eq!(
            get_secret("TEKMOR_TEST_SECRET").unwrap().as_deref(),
            Some("from-env")
        );
        std::env::remove_var("TEKMOR_TEST_SECRET");
    }
}
