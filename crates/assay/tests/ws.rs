mod common;

use common::run_lua_local;
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, Mutex};

async fn start_echo_ws_server() -> u16 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let ws_stream = tokio_tungstenite::accept_async(stream).await.unwrap();
                let (mut write, mut read) = ws_stream.split();
                while let Some(Ok(msg)) = read.next().await {
                    if (msg.is_text() || msg.is_binary()) && write.send(msg).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    port
}

#[derive(Default)]
struct CapturedHandshake {
    authorization: Option<String>,
    custom: Option<String>,
    requested_subprotocol: Option<String>,
}

// Echo server that also records the handshake request headers and negotiates a
// subprotocol back to the client (echoing the first requested protocol).
// tungstenite's Callback fixes the Err type to its large ErrorResponse, so the
// result-size lint is unavoidable here.
#[allow(clippy::result_large_err)]
async fn start_capturing_ws_server() -> (u16, Arc<Mutex<CapturedHandshake>>) {
    use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    let captured = Arc::new(Mutex::new(CapturedHandshake::default()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let cap = captured.clone();
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let cap = cap.clone();
            tokio::spawn(async move {
                let callback = |req: &Request,
                                mut response: Response|
                 -> Result<Response, ErrorResponse> {
                    let mut c = cap.lock().unwrap();
                    c.authorization = req
                        .headers()
                        .get("Authorization")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    c.custom = req
                        .headers()
                        .get("X-Custom")
                        .and_then(|v| v.to_str().ok())
                        .map(|s| s.to_string());
                    if let Some(sp) = req.headers().get("Sec-WebSocket-Protocol") {
                        let requested = sp.to_str().unwrap_or("").to_string();
                        let first = requested.split(',').next().unwrap_or("").trim().to_string();
                        c.requested_subprotocol = Some(requested);
                        response.headers_mut().insert(
                            "Sec-WebSocket-Protocol",
                            HeaderValue::from_str(&first).unwrap(),
                        );
                    }
                    Ok(response)
                };
                if let Ok(ws_stream) = tokio_tungstenite::accept_hdr_async(stream, callback).await {
                    let (mut write, mut read) = ws_stream.split();
                    while let Some(Ok(msg)) = read.next().await {
                        if (msg.is_text() || msg.is_binary()) && write.send(msg).await.is_err() {
                            break;
                        }
                    }
                }
            });
        }
    });
    (port, captured)
}

#[tokio::test]
async fn test_ws_connect_send_recv() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        ws.send(conn, "hello")
        local msg = ws.recv(conn)
        assert.eq(msg, "hello")
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ws_multiple_messages() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        ws.send(conn, "first")
        assert.eq(ws.recv(conn), "first")
        ws.send(conn, "second")
        assert.eq(ws.recv(conn), "second")
        ws.send(conn, "third")
        assert.eq(ws.recv(conn), "third")
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ws_close() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        ws.send(conn, "ping")
        assert.eq(ws.recv(conn), "ping")
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ws_connect_failure() {
    let result = run_lua_local(
        r#"
        ws.connect("ws://127.0.0.1:1/nonexistent")
    "#,
    )
    .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("ws.connect"),
        "error should mention ws.connect: {err}"
    );
}

#[tokio::test]
async fn test_ws_send_json() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        local payload = json.encode({{type = "ping", data = 42}})
        ws.send(conn, payload)
        local msg = ws.recv(conn)
        local parsed = json.parse(msg)
        assert.eq(parsed.type, "ping")
        assert.eq(parsed.data, 42)
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ws_connect_no_opts_backward_compat() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        assert.eq(ws.protocol(conn), nil)
        ws.send(conn, "hi")
        assert.eq(ws.recv(conn), "hi")
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}

#[tokio::test]
async fn test_ws_connect_subprotocol_and_headers() {
    let (port, captured) = start_capturing_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/", {{
            subprotocols = {{ "v4.channel.k8s.io" }},
            headers = {{
                Authorization = "Bearer test-token",
                ["X-Custom"] = "hello",
            }},
        }})
        assert.eq(ws.protocol(conn), "v4.channel.k8s.io")
        ws.send(conn, "ping")
        assert.eq(ws.recv(conn), "ping")
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();

    let c = captured.lock().unwrap();
    assert_eq!(c.authorization.as_deref(), Some("Bearer test-token"));
    assert_eq!(c.custom.as_deref(), Some("hello"));
    assert_eq!(
        c.requested_subprotocol.as_deref(),
        Some("v4.channel.k8s.io")
    );
}

#[tokio::test]
async fn test_ws_binary_roundtrip() {
    let port = start_echo_ws_server().await;
    run_lua_local(&format!(
        r#"
        local conn = ws.connect("ws://127.0.0.1:{port}/")
        -- Includes a non-UTF-8 byte (255) and an embedded NUL to prove the
        -- payload round-trips as raw bytes rather than a decoded UTF-8 string.
        local payload = "\1\2\3\255\0hello"
        ws.send_binary(conn, payload)
        local got = ws.recv(conn)
        assert.eq(got, payload)
        assert.eq(#got, 10)
        ws.close(conn)
    "#
    ))
    .await
    .unwrap();
}
