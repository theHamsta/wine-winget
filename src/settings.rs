use std::{fs::File, path::PathBuf};

use anyhow::{Context, Result, anyhow};
use log::{debug, info};
use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
pub struct Settings {
    pub repo_path: PathBuf,
}

impl Settings {
    pub fn read() -> Result<Self> {
        let config_path = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not get config dir"))?
            .join("wine-winget.yaml");
        debug!("Reading config file from {config_path:?}");
        let config_file = File::open(config_path).with_context(|| "Failed to open config file")?;
        Ok(yaml_serde::from_reader(config_file)?)
    }

    pub fn save(&self) -> Result<()> {
        let config_path = dirs::config_dir()
            .ok_or_else(|| anyhow!("Could not get config dir"))?
            .join("wine-winget.yaml");
        info!("Saving config file to {config_path:?}");
        println!("Saving config file to {config_path:?}");
        let config_file =
            File::create(config_path).with_context(|| "Failed to open config file")?;
        Ok(yaml_serde::to_writer(config_file, self)?)
    }
}
