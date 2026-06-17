//! Local adoption milestones (privacy-safe meta in brain.db).

use anyhow::Result;
use chrono::Utc;

use crate::db::store::BrainStore;

pub const INSTALLED_AT: &str = "adoption_installed_at";
pub const FIRST_ROUTE_AT: &str = "adoption_first_route_at";
pub const STARTER_PACK_AT: &str = "adoption_starter_pack_at";
pub const SUPERVISOR_PACK_AT: &str = "adoption_supervisor_pack_at";

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AdoptionMilestones {
    pub installed_at: Option<String>,
    pub first_route_at: Option<String>,
    pub starter_pack_at: Option<String>,
    pub supervisor_pack_at: Option<String>,
}

pub fn ensure_installed_at(store: &BrainStore) -> Result<()> {
    if store.get_meta(INSTALLED_AT)?.is_none() {
        store.set_meta(INSTALLED_AT, &Utc::now().to_rfc3339())?;
    }
    Ok(())
}

pub fn record_first_route(store: &BrainStore) -> Result<()> {
    if store.get_meta(FIRST_ROUTE_AT)?.is_none() {
        store.set_meta(FIRST_ROUTE_AT, &Utc::now().to_rfc3339())?;
    }
    Ok(())
}

pub fn record_starter_pack(store: &BrainStore) -> Result<()> {
    store.set_meta(STARTER_PACK_AT, &Utc::now().to_rfc3339())
}

pub fn record_supervisor_pack(store: &BrainStore) -> Result<()> {
    store.set_meta(SUPERVISOR_PACK_AT, &Utc::now().to_rfc3339())
}

pub fn load_milestones(store: &BrainStore) -> Result<AdoptionMilestones> {
    Ok(AdoptionMilestones {
        installed_at: store.get_meta(INSTALLED_AT)?,
        first_route_at: store.get_meta(FIRST_ROUTE_AT)?,
        starter_pack_at: store.get_meta(STARTER_PACK_AT)?,
        supervisor_pack_at: store.get_meta(SUPERVISOR_PACK_AT)?,
    })
}
