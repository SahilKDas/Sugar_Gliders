use rustls::pki_types::{CertificateDer, PrivatePkcs8KeyDer};
use std::{path::Path, sync::Arc, time::Duration};
pub type Error = Box<dyn std::error::Error + Send + Sync>;
pub const ALPN: &[u8] = b"maple/2";
pub const MAX_FRAME: usize = 1_000_000;
pub const MAX_COMMAND: usize = 16_384;
fn transport() -> Arc<quinn::TransportConfig> {
    let mut t = quinn::TransportConfig::default();
    t.max_concurrent_bidi_streams(1u8.into())
        .max_concurrent_uni_streams(0u8.into())
        .max_idle_timeout(Some(Duration::from_secs(30).try_into().unwrap()))
        .keep_alive_interval(Some(Duration::from_secs(5)))
        .datagram_receive_buffer_size(Some(64 * 1024))
        .datagram_send_buffer_size(64 * 1024);
    Arc::new(t)
}
/// Generate a local identity once. Never trust a certificate received from the network.
pub fn identity(directory: &Path) -> Result<(Vec<u8>, Vec<u8>), Error> {
    std::fs::create_dir_all(directory)?;
    let cert_path = directory.join("server.der");
    let key_path = directory.join("server.key");
    if cert_path.exists() || key_path.exists() {
        return Ok((std::fs::read(cert_path)?, std::fs::read(key_path)?));
    }
    let generated = rcgen::generate_simple_self_signed(vec!["localhost".into()])?;
    let cert = generated.cert.der().to_vec();
    let key = generated.key_pair.serialize_der();
    // create_new prevents another instance silently replacing this identity.
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(key_path)?;
    file.write_all(&key)?;
    file.sync_all()?;
    let mut file = options.open(cert_path)?;
    file.write_all(&cert)?;
    file.sync_all()?;
    Ok((cert, key))
}
pub fn server_config(cert: Vec<u8>, key: Vec<u8>) -> Result<quinn::ServerConfig, Error> {
    let mut tls = rustls::ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_no_client_auth()
    .with_single_cert(
        vec![CertificateDer::from(cert)],
        PrivatePkcs8KeyDer::from(key).into(),
    )?;
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.max_early_data_size = 0;
    let crypto = quinn::crypto::rustls::QuicServerConfig::try_from(tls)?;
    let mut config = quinn::ServerConfig::with_crypto(Arc::new(crypto));
    config.transport_config(transport());
    Ok(config)
}
pub fn client_config(cert: Vec<u8>) -> Result<quinn::ClientConfig, Error> {
    let mut roots = rustls::RootCertStore::empty();
    roots.add(CertificateDer::from(cert))?;
    let mut tls = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS13])?
    .with_root_certificates(roots)
    .with_no_client_auth();
    tls.alpn_protocols = vec![ALPN.to_vec()];
    tls.enable_early_data = false;
    let mut config = quinn::ClientConfig::new(Arc::new(
        quinn::crypto::rustls::QuicClientConfig::try_from(tls)?,
    ));
    config.transport_config(transport());
    Ok(config)
}
pub async fn write<T: serde::Serialize>(
    stream: &mut quinn::SendStream,
    value: &T,
) -> Result<(), Error> {
    let bytes = zeroize::Zeroizing::new(serde_json::to_vec(value)?);
    if bytes.len() > MAX_FRAME {
        return Err("Frame too large".into());
    }
    tokio::time::timeout(Duration::from_secs(5), async {
        stream
            .write_all(&(bytes.len() as u32).to_be_bytes())
            .await?;
        stream.write_all(&bytes).await
    })
    .await??;
    Ok(())
}
/// Cancellation-safe framing: run in a dedicated reader task, not directly in select!.
pub async fn read<T: serde::de::DeserializeOwned>(
    stream: &mut quinn::RecvStream,
    limit: usize,
) -> Result<T, Error> {
    let mut length = [0u8; 4];
    stream.read_exact(&mut length).await?;
    let length = u32::from_be_bytes(length) as usize;
    if length == 0 || length > limit {
        return Err("Invalid frame length".into());
    }
    let mut bytes = zeroize::Zeroizing::new(vec![0u8; length]);
    stream.read_exact(&mut bytes).await?;
    Ok(serde_json::from_slice(&bytes)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn rejects_expired_certificates() {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut params = rcgen::CertificateParams::new(vec!["localhost".into()]).unwrap();
        params.not_before = rcgen::date_time_ymd(2000, 1, 1);
        params.not_after = rcgen::date_time_ymd(2001, 1, 1);
        let cert = params.self_signed(&key).unwrap().der().to_vec();
        let server = quinn::Endpoint::server(
            server_config(cert.clone(), key.serialize_der()).unwrap(),
            "127.0.0.1:0".parse().unwrap(),
        )
        .unwrap();
        let accept = server.clone();
        let task = tokio::spawn(async move {
            if let Some(c) = accept.accept().await {
                let _ = c.await;
            }
        });
        let mut client = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        client.set_default_client_config(client_config(cert).unwrap());
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            client
                .connect(server.local_addr().unwrap(), "localhost")
                .unwrap(),
        )
        .await
        .unwrap();
        assert!(result.is_err());
        server.close(0u8.into(), b"Done");
        task.abort();
    }
}
