use anyhow::{Context, Result};
use p256::ecdsa::SigningKey;
use p256::pkcs8::DecodePrivateKey;
use rcgen::{CertificateParams, KeyPair};
use std::io::Write;
use std::time::Duration;
use tempfile::NamedTempFile;

use crate::config::Config;

/// Holds temporary PEM files for quiche TLS config and the pinned endpoint key.
pub struct TlsMaterial {
    pub cert_pem_file: NamedTempFile,
    pub key_pem_file: NamedTempFile,
    pub endpoint_pub_key_spki_der: Vec<u8>,
}

impl TlsMaterial {
    /// Load the client certificate and key into quiche, then delete both PEM
    /// files at once. Kept for the whole session, the key sat in /tmp, and the
    /// SIGTERM that stops the client skips destructors, so it stayed behind.
    /// Returns the pinned endpoint key.
    pub fn load_into(self, quic_config: &mut quiche::Config) -> Result<Vec<u8>> {
        let cert_path = self
            .cert_pem_file
            .path()
            .to_str()
            .context("temporary certificate path is not UTF-8")?;
        quic_config
            .load_cert_chain_from_pem_file(cert_path)
            .map_err(|e| anyhow::anyhow!("load cert: {e}"))?;
        let key_path = self
            .key_pem_file
            .path()
            .to_str()
            .context("temporary key path is not UTF-8")?;
        quic_config
            .load_priv_key_from_pem_file(key_path)
            .map_err(|e| anyhow::anyhow!("load key: {e}"))?;
        for file in [self.cert_pem_file, self.key_pem_file] {
            if let Err(error) = file.close() {
                log::warn!("failed to delete a temporary PEM file: {error}");
            }
        }
        Ok(self.endpoint_pub_key_spki_der)
    }
}

/// Generate self-signed client cert from the config private key and prepare
/// temp PEM files that quiche can load.
pub fn prepare_tls_material(config: &Config) -> Result<TlsMaterial> {
    let priv_key_der = config.get_ec_private_key_der()?;
    let signing_key = SigningKey::from_pkcs8_der(&priv_key_der)
        .context("failed to parse ECDSA private key from config")?;

    let key_pair_pem =
        p256::pkcs8::EncodePrivateKey::to_pkcs8_pem(&signing_key, p256::pkcs8::LineEnding::LF)
            .context("failed to encode private key to PEM")?;

    let key_pair =
        KeyPair::from_pem(key_pair_pem.as_ref()).context("failed to load key pair into rcgen")?;

    let mut params = CertificateParams::new(Vec::<String>::new())
        .context("failed to create certificate params")?;
    params.not_before = time::OffsetDateTime::now_utc();
    params.not_after = time::OffsetDateTime::now_utc() + Duration::from_secs(24 * 60 * 60);

    let cert = params
        .self_signed(&key_pair)
        .context("failed to generate self-signed certificate")?;

    let cert_pem = cert.pem();

    let mut cert_file = NamedTempFile::new().context("failed to create temp cert file")?;
    cert_file.write_all(cert_pem.as_bytes())?;
    cert_file.flush()?;

    let mut key_file = NamedTempFile::new().context("failed to create temp key file")?;
    key_file.write_all(key_pair_pem.as_bytes())?;
    key_file.flush()?;

    let endpoint_pub_key_spki_der = config.get_endpoint_pub_key_der()?;

    Ok(TlsMaterial {
        cert_pem_file: cert_file,
        key_pem_file: key_file,
        endpoint_pub_key_spki_der,
    })
}

/// Verify a peer's DER certificate against the pinned SPKI public key.
/// Returns true if the peer cert's `SubjectPublicKeyInfo` matches.
pub fn verify_endpoint_key(peer_cert_der: &[u8], expected_spki_der: &[u8]) -> bool {
    let Ok((_, cert)) = x509_parser::parse_x509_certificate(peer_cert_der) else {
        log::warn!("failed to parse peer certificate for key pinning");
        return false;
    };
    cert.tbs_certificate.subject_pki.raw == expected_spki_der
}

#[cfg(test)]
mod tests {
    use super::*;

    // A throwaway self-signed pair; the real key never enters the tests.
    fn material_with_test_cert() -> TlsMaterial {
        let key_pair = KeyPair::generate().expect("test key");
        let cert = CertificateParams::new(Vec::<String>::new())
            .expect("test params")
            .self_signed(&key_pair)
            .expect("test cert");
        let mut cert_pem_file = NamedTempFile::new().expect("temp cert");
        cert_pem_file
            .write_all(cert.pem().as_bytes())
            .expect("write cert");
        let mut key_pem_file = NamedTempFile::new().expect("temp key");
        key_pem_file
            .write_all(key_pair.serialize_pem().as_bytes())
            .expect("write key");
        TlsMaterial {
            cert_pem_file,
            key_pem_file,
            endpoint_pub_key_spki_der: vec![1, 2, 3],
        }
    }

    #[test]
    fn pem_files_are_deleted_once_quiche_has_loaded_them() {
        let material = material_with_test_cert();
        let paths = [
            material.cert_pem_file.path().to_path_buf(),
            material.key_pem_file.path().to_path_buf(),
        ];
        let mut quic_config = quiche::Config::new(quiche::PROTOCOL_VERSION).expect("config");
        let pin = material.load_into(&mut quic_config).expect("load");
        assert_eq!(pin, vec![1, 2, 3]);
        for path in &paths {
            assert!(!path.exists(), "{} still exists", path.display());
        }
    }
}
