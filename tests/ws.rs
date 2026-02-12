mod common;

use common::run_lua_local;
use futures_util::{SinkExt, StreamExt};

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
