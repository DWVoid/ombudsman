//! Configuration loading for ombudsman.

use crate::config::schema::Config;
use anyhow::{Context, Result};
use std::path::Path;

/// Load configuration from the default config file.
pub fn load_config() -> Result<Config> {
    let config_path = crate::config::paths::get_config_path();
    if config_path.exists() {
        load_config_from(&config_path)
    } else {
        Ok(Config::default())
    }
}

/// Load configuration from a specific path.
pub fn load_config_from(path: &Path) -> Result<Config> {
    let content = std::fs::read_to_string(path)
        .with_context(|| format!("Failed to read config file: {}", path.display()))?;
    let config: Config = serde_json::from_str(&content)
        .with_context(|| format!("Failed to parse config file: {}", path.display()))?;
    Ok(config)
}

/// Save configuration to the default config file.
pub fn save_config(config: &Config) -> Result<()> {
    let config_path = crate::config::paths::get_config_path();
    // Ensure parent directory exists
    if let Some(parent) = config_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create config directory: {}", parent.display()))?;
    }
    let content = serde_json::to_string_pretty(config)
        .context("Failed to serialize config")?;
    std::fs::write(&config_path, content)
        .with_context(|| format!("Failed to write config file: {}", config_path.display()))?;
    Ok(())
}
