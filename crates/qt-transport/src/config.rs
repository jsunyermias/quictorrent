use std::sync::Arc;
use quinn::{ClientConfig, ServerConfig};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

use crate::error::{Result, TransportError};

/// ALPN protocol id para bitturbulence.
/// Identifica el protocolo en el handshake TLS — los peers que no hablen
/// este protocolo serán rechazados automáticamente.
pub const ALPN_BITTURBULENCE: &[u8] = b"bitturbulence/1";

/// Genera un certificado TLS self-signed para un peer.
///
/// En BitTorrent sobre QUIC, la identidad del peer NO se deriva del cert
/// (a diferencia de mTLS clásico). El cert solo sirve para establecer el
/// canal cifrado; la autenticación real es a nivel de protocolo BT
/// (handshake con peer_id e info_hash).
pub fn generate_self_signed_cert() -> Result<(CertificateDer<'static>, PrivateKeyDer<'static>)> {
    let cert = rcgen::generate_simple_self_signed(vec!["bitturbulence".into()])
        .map_err(|e| TransportError::CertGen(e.to_string()))?;

    let cert_der = CertificateDer::from(cert.cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(
        cert.key_pair.serialize_der(),
    ));

    Ok((cert_der, key_der))
}

/// Configuración TLS para el lado servidor (acepta cualquier cliente).
pub fn server_config() -> Result<ServerConfig> {
    let (cert, key) = generate_self_signed_cert()?;

    let mut tls = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert], key)
        .map_err(|e| TransportError::Tls(e.to_string()))?;

    tls.alpn_protocols = vec![ALPN_BITTURBULENCE.to_vec()];

    let transport = default_transport_config();
    let mut cfg = ServerConfig::with_crypto(Arc::new(
        quinn::crypto::rustls::QuicServerConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(e.to_string()))?,
    ));
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

/// Configuración TLS para el lado cliente.
/// Acepta cualquier certificado del peer (trust-on-first-use).
pub fn client_config() -> Result<ClientConfig> {
    let mut tls = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(SkipServerVerification))
        .with_no_client_auth();

    tls.alpn_protocols = vec![ALPN_BITTURBULENCE.to_vec()];

    let transport = default_transport_config();
    let mut cfg = ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)
            .map_err(|e| TransportError::Tls(e.to_string()))?,
    ));
    cfg.transport_config(Arc::new(transport));
    Ok(cfg)
}

fn default_transport_config() -> quinn::TransportConfig {
    let mut t = quinn::TransportConfig::default();
    // Streams bidireccionales concurrentes máximos por conexión.
    // Cada pieza en vuelo usa un stream; 256 es conservador pero suficiente
    // para empezar. Se puede aumentar después de benchmarks.
    t.max_concurrent_bidi_streams(256u32.into());
    // Keep-alive cada 10s para mantener la conexión a través de NATs.
    t.keep_alive_interval(Some(std::time::Duration::from_secs(10)));
    // Timeout de idle: 30s sin actividad cierra la conexión.
    t.max_idle_timeout(Some(
        quinn::VarInt::from_u32(30_000).into(),
    ));
    t
}

/// Verificador TLS que acepta cualquier certificado.
/// La autenticación real se hace a nivel de protocolo BitTorrent.
#[derive(Debug)]
struct SkipServerVerification;

impl rustls::client::danger::ServerCertVerifier for SkipServerVerification {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn verify_tls13_signature(
        &self,
        _message: &[u8],
        _cert: &CertificateDer<'_>,
        _dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}
