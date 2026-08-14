//! Security integration tests for the MCP HTTP server.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use shapeport_core::config::RuntimeConfig;

// ---------------------------------------------------------------------------
// Helper: build a config with a specific token
// ---------------------------------------------------------------------------

fn config_with_token(token: &str) -> RuntimeConfig {
    let mut cfg = RuntimeConfig::default();
    cfg.mcp.bearer_token = Some(token.to_owned());
    cfg
}

fn config_with_origins(origins: Vec<String>) -> RuntimeConfig {
    let mut cfg = RuntimeConfig::default();
    cfg.mcp.origin_allowlist = origins;
    cfg
}

async fn wait_for_connect(addr: SocketAddr) -> std::io::Result<tokio::net::TcpStream> {
    let mut last_err = None;
    for _ in 0..20 {
        match tokio::net::TcpStream::connect(addr).await {
            Ok(stream) => return Ok(stream),
            Err(err) => {
                last_err = Some(err);
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            }
        }
    }
    Err(last_err.unwrap_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::ConnectionRefused,
            "server did not start",
        )
    }))
}

// ---------------------------------------------------------------------------
// Non-loopback without token → serve_http returns Err mentioning SHAPEPORT_MCP_TOKEN
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_loopback_without_token_returns_err() {
    // Use a non-loopback address (we don't actually bind – the check happens before bind).
    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0);
    let config = RuntimeConfig::default(); // no token set

    let err = shapeport_mcp::serve_http(addr, config)
        .await
        .expect_err("should fail without token on non-loopback");

    assert!(
        err.contains("SHAPEPORT_MCP_TOKEN"),
        "error should mention SHAPEPORT_MCP_TOKEN, got: {err}"
    );
}

// ---------------------------------------------------------------------------
// Loopback with origin allowlist: disallowed Origin → 403
// ---------------------------------------------------------------------------

#[tokio::test]
async fn origin_check_rejects_disallowed_origin() {
    let allowed_origin = "https://allowed.example";
    let evil_origin = "http://evil.example";

    // Bind on loopback with a restricted origin allowlist
    let mut cfg = config_with_origins(vec![allowed_origin.to_owned()]);
    cfg.mcp.bearer_token = Some("test-token".to_owned());

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind failed");
    let port = listener.local_addr().unwrap().port();
    drop(listener); // release so serve_http can re-bind

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

    // Spawn the server in the background
    tokio::spawn(async move {
        let _ = shapeport_mcp::serve_http(addr, cfg).await;
    });

    let client = wait_for_connect(addr).await.expect("connect failed");
    let (mut reader, mut writer) = client.into_split();

    let request = format!(
        "POST /mcp HTTP/1.1\r\n\
         Host: 127.0.0.1:{port}\r\n\
         Origin: {evil_origin}\r\n\
         Authorization: Bearer test-token\r\n\
         Content-Type: application/json\r\n\
         Content-Length: 2\r\n\
         \r\n\
         {{}}"
    );

    tokio::io::AsyncWriteExt::write_all(&mut writer, request.as_bytes())
        .await
        .expect("write failed");

    let mut response = Vec::new();
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::io::AsyncReadExt::read_buf(&mut reader, &mut response),
    )
    .await;

    let response_text = String::from_utf8_lossy(&response);
    assert!(
        response_text.starts_with("HTTP/1.1 403"),
        "expected 403 for disallowed origin, got: {response_text}"
    );
}

// ---------------------------------------------------------------------------
// Loopback without token: serve_http should succeed (no auth required)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn loopback_without_token_is_allowed() {
    // We only check that serve_http does not immediately return an error.
    // We cannot fully verify the server started without binding, so we use
    // the fact that an in-flight spawn does not panic.
    let config = config_with_token(""); // empty token effectively means none
    let mut cfg = RuntimeConfig::default();
    cfg.mcp.bearer_token = None;

    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind failed");
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);

    // serve_http on loopback without token should NOT return an early Err
    let handle = tokio::spawn(async move {
        // We abort quickly; the important thing is serve_http itself does not
        // immediately return Err.
        tokio::time::timeout(
            std::time::Duration::from_millis(200),
            shapeport_mcp::serve_http(addr, cfg),
        )
        .await
    });

    let result = handle.await.expect("task panicked");
    // timeout → Ok(Err(timeout)) meaning server was running (no immediate error)
    // immediate error → Ok(Ok(_)) or Ok(Err(_)) without timeout
    match result {
        Err(_timeout) => {
            // Server was still running after 200 ms – this is the success path
        }
        Ok(Err(err)) => {
            panic!("serve_http returned an early error on loopback: {err}");
        }
        Ok(Ok(())) => {
            // Server finished without error – also fine
        }
    }

    // Suppress unused warning from config_with_token
    drop(config);
}
