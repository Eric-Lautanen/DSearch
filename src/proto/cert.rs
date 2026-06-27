use ed25519_dalek::{SigningKey, VerifyingKey};
use ed25519_dalek::pkcs8::EncodePrivateKey;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::path::Path;

/// Generate an Ed25519 keypair and derive a self-signed TLS certificate
/// using the **same** key for both identity signing and TLS.
/// Returns (signing_key, node_id_hex, cert_der, key_pkcs8_der).
pub fn generate_identity() -> Result<(SigningKey, String, Vec<u8>, Vec<u8>), CertError> {
    let mut rng = rand::rngs::OsRng;
    let signing_key = SigningKey::generate(&mut rng);
    let node_id = node_id_from_pubkey(&signing_key.verifying_key());

    let (cert_der, key_der) = make_cert_and_key(&signing_key, &node_id)?;

    Ok((signing_key, node_id, cert_der, key_der))
}

/// Build a self-signed TLS cert + PKCS#8 private key DER from the signing key.
/// Uses the same Ed25519 keypair for both the certificate and TLS,
/// so the cert's public key always matches the private key.
fn make_cert_and_key(signing_key: &SigningKey, node_id: &str) -> Result<(Vec<u8>, Vec<u8>), CertError> {
    // Get PKCS#8 DER from the signing key (ed25519-dalek pkcs8 feature)
    let pkcs8_doc = signing_key.to_pkcs8_der()
        .map_err(|e| CertError::Generation(format!("PKCS#8 encode: {}", e)))?;
    let pkcs8_bytes = pkcs8_doc.as_bytes();

    // Build an rcgen KeyPair from the PKCS#8 DER — this ensures the rcgen
    // KeyPair uses the same Ed25519 key as our signing key.
    let key_pair = rcgen::KeyPair::from_pkcs8_der_and_sign_algo(
        &PrivatePkcs8KeyDer::from(pkcs8_bytes.to_vec()),
        &rcgen::PKCS_ED25519,
    ).map_err(|e| CertError::Generation(format!("rcgen KeyPair from PKCS#8: {}", e)))?;

    let mut params = rcgen::CertificateParams::default();
    params.distinguished_name.push(rcgen::DnType::CommonName, "dsearch");
    params.subject_alt_names.push(rcgen::SanType::URI(rcgen::Ia5String::try_from(node_id.to_string())
        .map_err(|e| CertError::Generation(format!("invalid SAN URI: {}", e)))?));
    let cert = params.self_signed(&key_pair)
        .map_err(|e| CertError::Generation(e.to_string()))?;

    Ok((cert.der().to_vec(), key_pair.serialize_der()))
}

/// Derive node_id as Blake3 hex of the Ed25519 public key bytes.
pub fn node_id_from_pubkey(verifying_key: &VerifyingKey) -> String {
    let pubkey_bytes = verifying_key.to_bytes();
    blake3::hash(&pubkey_bytes).to_hex().to_string()
}

/// Save identity key, TLS key, and cert to data_dir.
/// We store the raw Ed25519 secret key bytes (32 bytes) as identity.key,
/// the PKCS#8 TLS private key as identity.tls,
/// and the TLS cert DER as node.crt.
pub fn save_identity(data_dir: &Path, signing_key: &SigningKey, cert_der: &[u8], key_der: &[u8]) -> Result<(), CertError> {
    std::fs::create_dir_all(data_dir)
        .map_err(CertError::Io)?;

    let key_bytes = signing_key.to_bytes();
    std::fs::write(data_dir.join("identity.key"), &key_bytes)
        .map_err(CertError::Io)?;

    std::fs::write(data_dir.join("node.crt"), cert_der)
        .map_err(CertError::Io)?;

    std::fs::write(data_dir.join("identity.tls"), key_der)
        .map_err(CertError::Io)?;

    Ok(())
}

/// Load existing identity from data_dir, or generate and save a new one.
pub fn load_or_generate_identity(data_dir: &Path) -> Result<(SigningKey, String, Vec<u8>, Vec<u8>), CertError> {
    let key_path = data_dir.join("identity.key");
    let cert_path = data_dir.join("node.crt");
    let tls_key_path = data_dir.join("identity.tls");

    if key_path.exists() && cert_path.exists() && tls_key_path.exists() {
        let key_bytes = std::fs::read(&key_path).map_err(CertError::Io)?;
        let cert_der = std::fs::read(&cert_path).map_err(CertError::Io)?;
        let tls_key_der = std::fs::read(&tls_key_path).map_err(CertError::Io)?;

        let signing_key = SigningKey::from_bytes(
            key_bytes.as_slice().try_into()
                .map_err(|_| CertError::KeyFormat("identity.key must be 32 bytes".to_string()))?
        );
        let node_id = node_id_from_pubkey(&signing_key.verifying_key());

        // Verify the TLS key matches the signing key by re-deriving the key
        // and checking the PKCS#8 DER matches. If mismatch, re-issue cert+key.
        let (_, expected_key_der) = make_cert_and_key(&signing_key, &node_id)?;
        if tls_key_der != expected_key_der {
            // identity.tls doesn't match identity.key — re-issue both cert and key
            let (new_cert_der, new_key_der) = make_cert_and_key(&signing_key, &node_id)?;
            std::fs::write(&cert_path, &new_cert_der).map_err(CertError::Io)?;
            std::fs::write(&tls_key_path, &new_key_der).map_err(CertError::Io)?;
            return Ok((signing_key, node_id, new_cert_der, new_key_der));
        }

        Ok((signing_key, node_id, cert_der, tls_key_der))
    } else {
        // One or more files missing — regenerate everything
        let (signing_key, node_id, cert_der, key_der) = generate_identity()?;
        save_identity(data_dir, &signing_key, &cert_der, &key_der)?;
        Ok((signing_key, node_id, cert_der, key_der))
    }
}

/// Build a Quinn server configuration from cert_der and tls_key_der.
pub fn server_config(cert_der: &[u8], tls_key_der: &[u8]) -> Result<quinn::ServerConfig, CertError> {
    let cert = CertificateDer::from(cert_der.to_vec());
    let key = PrivateKeyDer::from(PrivatePkcs8KeyDer::from(tls_key_der.to_vec()));

    let tls_server_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let quic_server_config = quinn::crypto::rustls::QuicServerConfig::try_from(tls_server_config)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let mut server_config = quinn::ServerConfig::with_crypto(std::sync::Arc::new(quic_server_config));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(30).try_into().unwrap(),
    ));
    server_config.transport_config(std::sync::Arc::new(transport));

    Ok(server_config)
}

/// Build a Quinn client configuration with custom cert verifier.
pub fn client_config() -> Result<quinn::ClientConfig, CertError> {
    let tls_client_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(DsearchCertVerifier))
        .with_no_client_auth();

    let quic_client_config = quinn::crypto::rustls::QuicClientConfig::try_from(tls_client_config)
        .map_err(|e| CertError::Tls(e.to_string()))?;

    let mut client_config = quinn::ClientConfig::new(std::sync::Arc::new(quic_client_config));
    let mut transport = quinn::TransportConfig::default();
    transport.max_idle_timeout(Some(
        std::time::Duration::from_secs(30).try_into().unwrap(),
    ));
    client_config.transport_config(std::sync::Arc::new(transport));

    Ok(client_config)
}

/// Custom certificate verifier that accepts self-signed dsearch certs.
/// The real authentication happens at the protocol level (node_id in handshake).
#[derive(Debug)]
struct DsearchCertVerifier;

impl rustls::client::danger::ServerCertVerifier for DsearchCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &rustls::pki_types::CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        vec![rustls::SignatureScheme::ED25519]
    }
}

#[derive(Debug)]
pub enum CertError {
    Generation(String),
    Tls(String),
    KeyFormat(String),
    Io(std::io::Error),
}

impl std::fmt::Display for CertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CertError::Generation(s) => write!(f, "cert generation error: {}", s),
            CertError::Tls(s) => write!(f, "TLS error: {}", s),
            CertError::KeyFormat(s) => write!(f, "key format error: {}", s),
            CertError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for CertError {}
