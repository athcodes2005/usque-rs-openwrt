use anyhow::{Context, Result};
use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::Path;
use tempfile::NamedTempFile;

use crate::register::AccountData;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub private_key: String,
    pub endpoint_v4: String,
    pub endpoint_v6: String,
    pub endpoint_pub_key: String,
    pub license: String,
    pub id: String,
    pub access_token: String,
    pub ipv4: String,
    pub ipv6: String,
}

impl Config {
    pub fn load(path: &str) -> Result<Self> {
        let data = fs::read_to_string(path)
            .with_context(|| format!("failed to read config from {path}"))?;
        serde_json::from_str(&data).with_context(|| "failed to parse config JSON")
    }

    pub fn save(&self, path: &str) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        let target = Path::new(path);
        if let Some(parent) = target.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }

        let parent = target
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let mut temp = NamedTempFile::new_in(parent)
            .with_context(|| format!("failed to create temporary config beside {path}"))?;
        temp.write_all(json.as_bytes())?;
        temp.flush()?;
        temp.as_file().sync_all()?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            temp.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))
                .with_context(|| format!("failed to secure config permissions for {path}"))?;
        }

        temp.persist(target)
            .map_err(|error| error.error)
            .with_context(|| format!("failed to atomically replace config at {path}"))?;

        Ok(())
    }

    pub fn from_account_data(
        account: &AccountData,
        token: &str,
        priv_key_der: &[u8],
    ) -> Result<Self> {
        let peer = account
            .config
            .peers
            .first()
            .context("registration response did not contain a WARP peer")?;
        let ep_v4 = peer.endpoint.v4.trim_end_matches(":0").to_string();
        let ep_v6 = peer.endpoint.v6.clone();
        let ep_v6 = ep_v6
            .trim_start_matches('[')
            .trim_end_matches("]:0")
            .trim_end_matches(":0")
            .to_string();

        Ok(Self {
            private_key: base64::engine::general_purpose::STANDARD.encode(priv_key_der),
            endpoint_v4: ep_v4,
            endpoint_v6: ep_v6,
            endpoint_pub_key: peer.public_key.clone(),
            license: account.account.license.clone().unwrap_or_default(),
            id: account.id.clone(),
            access_token: token.to_string(),
            ipv4: account.config.interface.addresses.v4.clone(),
            ipv6: account.config.interface.addresses.v6.clone(),
        })
    }

    pub fn get_ec_private_key_der(&self) -> Result<Vec<u8>> {
        base64::engine::general_purpose::STANDARD
            .decode(&self.private_key)
            .with_context(|| "failed to decode private key from base64")
    }

    pub fn get_endpoint_pub_key_der(&self) -> Result<Vec<u8>> {
        let pem = pem::parse(&self.endpoint_pub_key)
            .with_context(|| "failed to parse endpoint public key PEM")?;
        Ok(pem.contents().to_vec())
    }
}
