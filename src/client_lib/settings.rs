use std::{env, error::Error, net::Ipv4Addr};

use config::Config;
use serde::Deserialize;

#[derive(Deserialize, Debug)]
pub struct Settings {
    addr: Ipv4Addr,
    port: u16,
}

impl Settings {
    pub fn get_full_address(&self) -> String {
        format!("{}:{}", self.addr, self.port)
    }
}

pub fn get_settings() -> Result<Settings, Box<dyn Error>> {
    let path = env::current_dir()?
        .join("configuration")
        .join("ClientSettings.toml");
    let settings = Config::builder()
        .add_source(config::File::from(path))
        .build()?;

    Ok(settings.try_deserialize()?)
}
