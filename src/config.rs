use anyhow::{anyhow, Context, Error};
use serde::{Deserialize, Serialize};
use std::env;
use std::path::Path;
use toml;

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub terraforming_path: String,
    pub kubernetes_deployments_path: String,
    pub keybase_secure_manifests_path: String,
    pub kubernetes_deployments_revision: String,
    pub kubernetes_deployments_ssh_key: String,
    pub infra_profile: String,
    pub v1_profile: String,
    pub assets_cache_path: String,
}

impl Config {
    pub fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Error> {
        let s = std::fs::read_to_string(&path).context("config file not found")?;
        toml::from_str(&s).context("config parsing error")
    }
}
