//! Proxy-aware outgoing HTTP for Wasm providers.
//!
//! `wasmtime-wasi-http`'s default outgoing handler opens a direct TCP
//! connection to the target authority and ignores the standard
//! `HTTP_PROXY` / `HTTPS_PROXY` / `ALL_PROXY` environment variables. In
//! environments where direct egress is blocked and all traffic must go
//! through a forward proxy (which is exactly the case for the sandbox this
//! project is developed in), every provider request then fails with a TLS
//! error because the first bytes spoken to the intercepting gateway are a
//! TLS ClientHello instead of an HTTP `CONNECT`.
//!
//! This module overrides `WasiHttpView::send_request`: when a proxy is
//! configured for the target host it tunnels the connection through the
//! proxy with `CONNECT` and then runs the same TLS/HTTP1 exchange as the
//! default handler on top of the tunnel; otherwise it delegates to
//! `wasmtime_wasi_http::types::default_send_request` unchanged.

use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use wasmtime_wasi_http::bindings::http::types::ErrorCode;
use wasmtime_wasi_http::body::HyperOutgoingBody;
use wasmtime_wasi_http::io::TokioIo;
use wasmtime_wasi_http::types::{
    default_send_request, HostFutureIncomingResponse, IncomingResponse, OutgoingRequestConfig,
};
use wasmtime_wasi_http::{hyper_request_error, HttpResult};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProxyConfig {
    /// `host:port` of the forward proxy.
    pub host_port: String,
    /// Full `Proxy-Authorization` header value (e.g. `Basic ...`), if the
    /// proxy URL carried credentials. Never logged.
    pub authorization: Option<String>,
}

/// Parse an HTTP proxy URL of the form `http://[user[:pass]@]host[:port]`.
/// Returns `None` for non-HTTP schemes or unparseable values.
pub fn parse_proxy_url(raw: &str) -> Option<ProxyConfig> {
    let rest = raw.trim().strip_prefix("http://")?;
    let authority = rest.split('/').next()?;
    if authority.is_empty() {
        return None;
    }

    let (userinfo, host_port) = match authority.rsplit_once('@') {
        Some((user, host)) => (Some(user), host),
        None => (None, authority),
    };
    if host_port.is_empty() {
        return None;
    }
    let host_port = if host_port.contains(':') {
        host_port.to_string()
    } else {
        format!("{host_port}:80")
    };

    let authorization = userinfo.map(|info| {
        let decoded = percent_decode(info);
        format!("Basic {}", base64_encode(decoded.as_bytes()))
    });

    Some(ProxyConfig {
        host_port,
        authorization,
    })
}

fn percent_decode(input: &str) -> String {
    percent_encoding::percent_decode_str(input)
        .decode_utf8_lossy()
        .into_owned()
}

fn base64_encode(data: &[u8]) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(data)
}

/// Whether `host` is covered by a `NO_PROXY` list (comma separated;
/// entries may be exact hosts, suffixes with or without a leading dot,
/// or `*`).
pub fn host_matches_no_proxy(host: &str, no_proxy: &str) -> bool {
    let host = host
        .trim()
        .trim_matches(|c| c == '[' || c == ']')
        .to_lowercase();
    for entry in no_proxy.split(',') {
        let entry = entry.trim().to_lowercase();
        if entry.is_empty() {
            continue;
        }
        if entry == "*" || entry == host {
            return true;
        }
        let suffix = entry.strip_prefix('.').unwrap_or(&entry);
        if host == suffix || host.ends_with(&format!(".{suffix}")) {
            return true;
        }
    }
    false
}

/// Resolve the proxy configured for a target host, honouring
/// `HTTPS_PROXY`/`HTTP_PROXY` (scheme dependent), `ALL_PROXY`, and
/// `NO_PROXY`. Returns `None` when requests should go direct.
pub fn proxy_for(use_tls: bool, target_host: &str) -> Option<ProxyConfig> {
    let no_proxy = std::env::var("NO_PROXY")
        .ok()
        .or_else(|| std::env::var("no_proxy").ok())
        .unwrap_or_default();
    if host_matches_no_proxy(target_host, &no_proxy) {
        return None;
    }

    let keys: &[&str] = if use_tls {
        &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
    } else {
        &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"]
    };
    for key in keys {
        if let Ok(val) = std::env::var(key) {
            if !val.trim().is_empty() {
                return parse_proxy_url(&val);
            }
        }
    }
    None
}

/// `WasiHttpView::send_request` implementation: proxy tunnel when a
/// proxy applies to the request authority, default handler otherwise.
pub fn send_request(
    request: hyper::Request<HyperOutgoingBody>,
    config: OutgoingRequestConfig,
) -> HttpResult<HostFutureIncomingResponse> {
    let target_host = request
        .uri()
        .host()
        .unwrap_or_default()
        .trim_matches(|c| c == '[' || c == ']')
        .to_string();

    let Some(proxy) = proxy_for(config.use_tls, &target_host) else {
        return Ok(default_send_request(request, config));
    };

    let handle =
        wasmtime_wasi::runtime::spawn(
            async move { Ok(send_via_proxy(request, config, proxy).await) },
        );
    Ok(HostFutureIncomingResponse::pending(handle))
}

/// Run the HTTP/1.1 client handshake over a (possibly TLS) stream and
/// spawn the connection driver, as in the default outgoing handler.
async fn http1_handshake<S>(
    stream: S,
    connect_timeout: std::time::Duration,
) -> Result<
    (
        hyper::client::conn::http1::SendRequest<HyperOutgoingBody>,
        wasmtime_wasi::runtime::AbortOnDropJoinHandle<()>,
    ),
    ErrorCode,
>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let io = TokioIo::new(stream);
    let (sender, conn) = timeout(connect_timeout, hyper::client::conn::http1::handshake(io))
        .await
        .map_err(|_| ErrorCode::ConnectionTimeout)?
        .map_err(hyper_request_error)?;
    let worker = wasmtime_wasi::runtime::spawn(async move {
        let _ = conn.await;
    });
    Ok((sender, worker))
}

async fn send_via_proxy(
    mut request: hyper::Request<HyperOutgoingBody>,
    config: OutgoingRequestConfig,
    proxy: ProxyConfig,
) -> Result<IncomingResponse, ErrorCode> {
    let authority = if let Some(authority) = request.uri().authority() {
        if authority.port().is_some() {
            authority.to_string()
        } else {
            let port = if config.use_tls { 443 } else { 80 };
            format!("{authority}:{port}")
        }
    } else {
        return Err(ErrorCode::HttpRequestUriInvalid);
    };

    // Connect to the proxy and establish a CONNECT tunnel.
    let mut stream = timeout(config.connect_timeout, TcpStream::connect(&proxy.host_port))
        .await
        .map_err(|_| ErrorCode::ConnectionTimeout)?
        .map_err(|_| ErrorCode::ConnectionRefused)?;

    let mut connect_req = format!(
        "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Connection: keep-alive\r\n"
    );
    if let Some(auth) = &proxy.authorization {
        connect_req.push_str(&format!("Proxy-Authorization: {auth}\r\n"));
    }
    connect_req.push_str("\r\n");
    stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(|_| ErrorCode::ConnectionRefused)?;

    // Read the proxy response header block byte-by-byte so no tunnelled
    // payload bytes are consumed past the header terminator.
    let mut header = Vec::with_capacity(256);
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() > 16 * 1024 {
            return Err(ErrorCode::HttpProtocolError);
        }
        let n = timeout(config.connect_timeout, stream.read(&mut byte))
            .await
            .map_err(|_| ErrorCode::ConnectionTimeout)?
            .map_err(|_| ErrorCode::ConnectionRefused)?;
        if n == 0 {
            return Err(ErrorCode::ConnectionRefused);
        }
        header.push(byte[0]);
    }
    let header_text = String::from_utf8_lossy(&header);
    let status: u32 = header_text
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    if status != 200 {
        return Err(ErrorCode::ConnectionRefused);
    }

    // Same TLS / plain HTTP1 exchange as the default handler, now over
    // the tunnelled TCP stream.
    let (mut sender, worker) = if config.use_tls {
        use rustls::pki_types::ServerName;

        // WebPKI roots plus the OS native store: behind a TLS-intercepting
        // forward proxy the tunnelled connection presents a certificate
        // signed by the proxy's CA, which is installed in the system store
        // (e.g. /etc/ssl/certs) but is not a WebPKI root.
        let mut root_cert_store = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.into(),
        };
        for cert in rustls_native_certs::load_native_certs().certs {
            let _ = root_cert_store.add(cert);
        }
        let tls_config = rustls::ClientConfig::builder()
            .with_root_certificates(root_cert_store)
            .with_no_client_auth();
        let connector = tokio_rustls::TlsConnector::from(std::sync::Arc::new(tls_config));
        let host = authority.split(':').next().unwrap_or(&authority);
        let domain = ServerName::try_from(host)
            .map_err(|_| ErrorCode::DestinationNotFound)?
            .to_owned();
        let tls_stream = match connector.connect(domain, stream).await {
            Ok(s) => s,
            Err(e) => {
                eprintln!("wallpp: TLS handshake failed for {host}: {e:?}");
                return Err(ErrorCode::TlsProtocolError);
            }
        };
        http1_handshake(tls_stream, config.connect_timeout).await?
    } else {
        http1_handshake(stream, config.connect_timeout).await?
    };

    // The request on the tunnel is origin-form, as in the default handler.
    *request.uri_mut() = http::Uri::builder()
        .path_and_query(
            request
                .uri()
                .path_and_query()
                .map(|p| p.as_str())
                .unwrap_or("/"),
        )
        .build()
        .expect("comes from valid request");

    let resp = timeout(config.first_byte_timeout, sender.send_request(request))
        .await
        .map_err(|_| ErrorCode::ConnectionReadTimeout)?
        .map_err(hyper_request_error)?
        .map(|body| body.map_err(hyper_request_error).boxed());

    Ok(IncomingResponse {
        resp,
        worker: Some(worker),
        between_bytes_timeout: config.between_bytes_timeout,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_proxy_with_credentials() {
        let p = parse_proxy_url("http://user:p%40ss@proxy.example:3128").unwrap();
        assert_eq!(p.host_port, "proxy.example:3128");
        // "user:p@ss" base64
        assert_eq!(
            p.authorization.as_deref(),
            Some(format!("Basic {}", base64_encode(b"user:p@ss")).as_str())
        );
    }

    #[test]
    fn parses_proxy_without_credentials_or_port() {
        let p = parse_proxy_url("http://proxy.example").unwrap();
        assert_eq!(p.host_port, "proxy.example:80");
        assert_eq!(p.authorization, None);
    }

    #[test]
    fn rejects_non_http_proxy() {
        assert!(parse_proxy_url("socks5://127.0.0.1:1080").is_none());
        assert!(parse_proxy_url("proxy.example:3128").is_none());
    }

    #[test]
    fn no_proxy_matching() {
        assert!(host_matches_no_proxy("example.com", "example.com"));
        assert!(host_matches_no_proxy("api.example.com", ".example.com"));
        assert!(host_matches_no_proxy("api.example.com", "example.com"));
        assert!(host_matches_no_proxy("anything", "*"));
        assert!(!host_matches_no_proxy("example.org", "example.com"));
        assert!(!host_matches_no_proxy("example.com", ""));
    }
}
