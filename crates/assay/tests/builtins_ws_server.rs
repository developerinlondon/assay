//! Server-side WebSocket upgrade tests for `http.serve` and the
//! `assay.shell.bridge` integration on top of it.
//!
//! These cover the full stack: hyper upgrade handshake, `WsServerConn`
//! UserData methods (read/write/close), the `process.spawn_pty` PTY plumbing,
//! and the resize control protocol in `assay.shell`.

mod common;

use common::create_vm;
use futures_util::{SinkExt, StreamExt};
use mlua::Lua;
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

async fn wait_for_port(lua: &Lua) -> u16 {
    for _ in 0..200 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if let Ok(p) = lua.globals().get::<u16>("_SERVER_PORT")
            && p > 0
        {
            return p;
        }
    }
    panic!("server didn't bind within 2s");
}

fn run<F>(body: F)
where
    F: std::future::Future<Output = ()> + 'static,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let local = tokio::task::LocalSet::new();
    local.block_on(&rt, body);
}

#[test]
fn ws_text_echo() {
    run(async {
        let lua = create_vm();
        let lua_for_serve = lua.clone();
        tokio::task::spawn_local(async move {
            lua_for_serve
                .load(
                    r#"
                    http.serve(0, {
                        GET = {
                            ["/echo"] = function(req)
                                return { ws = function(conn)
                                    while true do
                                        local m = conn:read()
                                        if not m then break end
                                        conn:write(m)
                                    end
                                end }
                            end,
                        },
                    })
                "#,
                )
                .exec_async()
                .await
                .ok();
        });

        let port = wait_for_port(&lua).await;
        let url = format!("ws://127.0.0.1:{port}/echo");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
        ws.send(Message::Text("hello".into())).await.unwrap();
        let got = ws.next().await.unwrap().unwrap();
        match got {
            Message::Text(t) => assert_eq!(t.as_str(), "hello"),
            other => panic!("expected text frame, got {other:?}"),
        }
        ws.close(None).await.ok();
    });
}

#[test]
fn ws_binary_echo() {
    run(async {
        let lua = create_vm();
        let lua_for_serve = lua.clone();
        tokio::task::spawn_local(async move {
            lua_for_serve
                .load(
                    r#"
                    http.serve(0, {
                        GET = {
                            ["/echo"] = function(req)
                                return { ws = function(conn)
                                    local m = conn:read()
                                    if m then conn:write(m, { binary = true }) end
                                end }
                            end,
                        },
                    })
                "#,
                )
                .exec_async()
                .await
                .ok();
        });

        let port = wait_for_port(&lua).await;
        let url = format!("ws://127.0.0.1:{port}/echo");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let payload: Vec<u8> = vec![0xde, 0xad, 0xbe, 0xef, 0x00, 0xff];
        ws.send(Message::Binary(payload.clone().into()))
            .await
            .unwrap();
        let got = ws.next().await.unwrap().unwrap();
        match got {
            Message::Binary(b) => assert_eq!(b.as_ref(), payload.as_slice()),
            other => panic!("expected binary frame, got {other:?}"),
        }
        ws.close(None).await.ok();
    });
}

#[test]
fn ws_server_initiated_close() {
    run(async {
        let lua = create_vm();
        let lua_for_serve = lua.clone();
        tokio::task::spawn_local(async move {
            lua_for_serve
                .load(
                    r#"
                    http.serve(0, {
                        GET = {
                            ["/bye"] = function(req)
                                return { ws = function(conn)
                                    conn:write("greeting")
                                    conn:close(1000, "bye")
                                end }
                            end,
                        },
                    })
                "#,
                )
                .exec_async()
                .await
                .ok();
        });

        let port = wait_for_port(&lua).await;
        let url = format!("ws://127.0.0.1:{port}/bye");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();
        let greet = ws.next().await.unwrap().unwrap();
        assert!(matches!(greet, Message::Text(ref t) if t.as_str() == "greeting"));
        let close = ws.next().await.unwrap().unwrap();
        match close {
            Message::Close(Some(cf)) => {
                assert_eq!(u16::from(cf.code), 1000);
                assert_eq!(cf.reason.as_str(), "bye");
            }
            other => panic!("expected close frame, got {other:?}"),
        }
    });
}

#[test]
fn ws_concurrent_clients() {
    run(async {
        let lua = create_vm();
        let lua_for_serve = lua.clone();
        tokio::task::spawn_local(async move {
            lua_for_serve
                .load(
                    r#"
                    http.serve(0, {
                        GET = {
                            ["/id"] = function(req)
                                return { ws = function(conn)
                                    while true do
                                        local m = conn:read()
                                        if not m then break end
                                        conn:write("got:" .. m)
                                    end
                                end }
                            end,
                        },
                    })
                "#,
                )
                .exec_async()
                .await
                .ok();
        });

        let port = wait_for_port(&lua).await;
        let url = format!("ws://127.0.0.1:{port}/id");

        let mut handles = Vec::new();
        for i in 0..5 {
            let url = url.clone();
            handles.push(tokio::task::spawn_local(async move {
                let (mut ws, _) = tokio_tungstenite::connect_async(&url).await.unwrap();
                let msg = format!("client-{i}");
                ws.send(Message::Text(msg.clone().into())).await.unwrap();
                let got = ws.next().await.unwrap().unwrap();
                match got {
                    Message::Text(t) => {
                        assert_eq!(t.as_str(), format!("got:{msg}"));
                    }
                    other => panic!("expected text frame, got {other:?}"),
                }
            }));
        }
        for h in handles {
            h.await.unwrap();
        }
    });
}

/// End-to-end: assay.shell.bridge a websocket to a `cat` PTY, exchange bytes,
/// verify the resize control frame is consumed (not echoed back).
#[test]
fn shell_bridge_to_cat_pty() {
    run(async {
        let lua = create_vm();
        let lua_for_serve = lua.clone();
        tokio::task::spawn_local(async move {
            lua_for_serve
                .load(
                    r#"
                    local shell = require("assay.shell")
                    http.serve(0, {
                        GET = {
                            ["/shell"] = function(req)
                                return { ws = function(conn)
                                    shell.bridge(conn, { cmd = "cat" })
                                end }
                            end,
                        },
                    })
                "#,
                )
                .exec_async()
                .await
                .ok();
        });

        let port = wait_for_port(&lua).await;
        let url = format!("ws://127.0.0.1:{port}/shell");
        let (mut ws, _resp) = tokio_tungstenite::connect_async(&url).await.unwrap();

        // Resize control frame — should be consumed by the bridge, not echoed.
        ws.send(Message::Text(
            r#"{"resize":{"cols":120,"rows":40}}"#.into(),
        ))
        .await
        .unwrap();

        // Send actual bytes — cat should echo them back as binary frames
        // (the bridge always uses opts.binary=true on PTY -> ws).
        ws.send(Message::Binary(b"ping\n".to_vec().into()))
            .await
            .unwrap();

        // Read back; expect to see "ping" within a few frames.
        let mut got = Vec::<u8>::new();
        for _ in 0..20 {
            let frame = tokio::time::timeout(Duration::from_secs(2), ws.next())
                .await
                .expect("frame timeout")
                .unwrap()
                .unwrap();
            match frame {
                Message::Binary(b) => {
                    got.extend_from_slice(&b);
                    if got.windows(4).any(|w| w == b"ping") {
                        break;
                    }
                }
                Message::Text(t) => {
                    // The resize frame must NOT have come back as an echo.
                    assert!(
                        !t.as_str().contains("\"resize\""),
                        "resize control frame leaked back to client: {t:?}"
                    );
                }
                _ => {}
            }
        }
        assert!(
            got.windows(4).any(|w| w == b"ping"),
            "didn't see 'ping' in PTY output: {got:?}"
        );

        ws.close(None).await.ok();
    });
}
