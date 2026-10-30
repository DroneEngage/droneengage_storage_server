//! TLS and WebSocket settings for the listeners.

use rustls::crypto::{ring, CryptoProvider};
use rustls_pki_types::pem::PemObject;
use rustls_pki_types::{CertificateDer, PrivateKeyDer};
use std::path::Path;
use std::sync::Arc;
use tokio_rustls::TlsAcceptor;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// ws's default maxPayload (100 MiB).
pub const MAX_PAYLOAD: usize = 100 * 1024 * 1024;

pub fn ws_config() -> WebSocketConfig {
    WebSocketConfig::default().max_message_size(Some(MAX_PAYLOAD)).max_frame_size(Some(MAX_PAYLOAD))
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(ring::default_provider())
}

/// A TLS acceptor for a key/cert pair (PEM).
pub fn tls_acceptor(p_key: &Path, p_cert: &Path) -> Result<TlsAcceptor, String> {
    let v_certs = CertificateDer::pem_file_iter(p_cert)
        .and_then(|it| it.collect::<Result<Vec<_>, _>>())
        .map_err(|e| format!("invalid SSL cert file {}: {}", p_cert.display(), e))?;
    let v_key =
        PrivateKeyDer::from_pem_file(p_key).map_err(|e| format!("invalid SSL key file {}: {}", p_key.display(), e))?;
    let v_server = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?
        .with_no_client_auth()
        .with_single_cert(v_certs, v_key)
        .map_err(|e| format!("SSL certificate/key rejected: {}", e))?;
    Ok(TlsAcceptor::from(Arc::new(v_server)))
}
