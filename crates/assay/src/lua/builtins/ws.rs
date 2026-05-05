use futures_util::{SinkExt, StreamExt};
use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, frame::coding::CloseCode};

type WsSink = Rc<
    tokio::sync::Mutex<
        futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
            tokio_tungstenite::tungstenite::Message,
        >,
    >,
>;
type WsStream = Rc<
    tokio::sync::Mutex<
        futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
        >,
    >,
>;

struct WsConn {
    sink: WsSink,
    stream: WsStream,
}
impl UserData for WsConn {}

// Server-side WebSocket connection. The underlying stream comes from a
// hyper upgrade and so its socket type is `TokioIo<hyper::upgrade::Upgraded>`,
// distinct from the client `MaybeTlsStream<TcpStream>`. Methods are exposed
// directly on the userdata (`conn:read()`, etc.) rather than through a `ws`
// table, since handlers receive the connection by callback.
pub type ServerWsStream =
    tokio_tungstenite::WebSocketStream<hyper_util::rt::TokioIo<hyper::upgrade::Upgraded>>;
pub type ServerWsSink =
    Rc<tokio::sync::Mutex<futures_util::stream::SplitSink<ServerWsStream, Message>>>;
pub type ServerWsRead = Rc<tokio::sync::Mutex<futures_util::stream::SplitStream<ServerWsStream>>>;

pub struct WsServerConn {
    sink: ServerWsSink,
    stream: ServerWsRead,
    peer_addr: String,
    closed: Rc<AtomicBool>,
}

impl WsServerConn {
    pub fn new(stream: ServerWsStream, peer_addr: String) -> Self {
        let (sink, read) = stream.split();
        Self {
            sink: Rc::new(tokio::sync::Mutex::new(sink)),
            stream: Rc::new(tokio::sync::Mutex::new(read)),
            peer_addr,
            closed: Rc::new(AtomicBool::new(false)),
        }
    }
}

impl UserData for WsServerConn {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("peer_addr", |_, c| Ok(c.peer_addr.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_async_method("read", |lua, this, ()| async move {
            let stream = this.stream.clone();
            let closed = this.closed.clone();
            loop {
                let next = stream.lock().await.next().await;
                match next {
                    None => {
                        closed.store(true, Ordering::Release);
                        return Ok(Value::Nil);
                    }
                    Some(Err(e)) => {
                        closed.store(true, Ordering::Release);
                        return Err(mlua::Error::runtime(format!("conn:read: {e}")));
                    }
                    Some(Ok(Message::Text(t))) => {
                        return Ok(Value::String(lua.create_string(t.as_bytes())?));
                    }
                    Some(Ok(Message::Binary(b))) => {
                        return Ok(Value::String(lua.create_string(&b)?));
                    }
                    Some(Ok(Message::Close(_))) => {
                        closed.store(true, Ordering::Release);
                        return Ok(Value::Nil);
                    }
                    // Ping/Pong/Frame: handled by tungstenite; loop for next message.
                    Some(Ok(_)) => continue,
                }
            }
        });

        methods.add_async_method(
            "write",
            |_, this, (data, opts): (mlua::String, Option<Table>)| async move {
                let binary = opts
                    .as_ref()
                    .and_then(|t| t.get::<Option<bool>>("binary").ok().flatten())
                    .unwrap_or(false);
                let bytes = data.as_bytes().to_vec();
                let msg = if binary {
                    Message::Binary(bytes.into())
                } else {
                    let text = String::from_utf8(bytes).map_err(|e| {
                        mlua::Error::runtime(format!(
                            "conn:write: text frames must be UTF-8 (use opts.binary=true for raw bytes): {e}"
                        ))
                    })?;
                    Message::Text(text.into())
                };
                this.sink
                    .lock()
                    .await
                    .send(msg)
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("conn:write: {e}")))?;
                Ok(())
            },
        );

        methods.add_async_method(
            "close",
            |_, this, (code, reason): (Option<u16>, Option<String>)| async move {
                if this.closed.swap(true, Ordering::AcqRel) {
                    return Ok(());
                }
                let close_frame = match (code, reason) {
                    (None, None) => None,
                    (code, reason) => Some(CloseFrame {
                        code: CloseCode::from(code.unwrap_or(1000)),
                        reason: reason.unwrap_or_default().into(),
                    }),
                };
                let mut sink = this.sink.lock().await;
                let _ = sink.send(Message::Close(close_frame)).await;
                let _ = sink.close().await;
                Ok(())
            },
        );

        methods.add_method("is_closed", |_, this, ()| {
            Ok(this.closed.load(Ordering::Acquire))
        });
    }
}

fn extract_ws_conn(val: &Value, fn_name: &str) -> mlua::Result<(WsSink, WsStream)> {
    let ud = match val {
        Value::UserData(ud) => ud,
        _ => {
            return Err(mlua::Error::runtime(format!(
                "{fn_name}: first argument must be a ws connection"
            )));
        }
    };
    let ws = ud.borrow::<WsConn>().map_err(|_| {
        mlua::Error::runtime(format!("{fn_name}: first argument must be a ws connection"))
    })?;
    Ok((ws.sink.clone(), ws.stream.clone()))
}

pub fn register_ws(lua: &Lua) -> mlua::Result<()> {
    let ws_table = lua.create_table()?;

    let connect_fn = lua.create_async_function(|lua, url: String| async move {
        let (stream, _response) = tokio_tungstenite::connect_async(&url)
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.connect: {e}")))?;
        let (sink, read) = stream.split();
        lua.create_any_userdata(WsConn {
            sink: Rc::new(tokio::sync::Mutex::new(sink)),
            stream: Rc::new(tokio::sync::Mutex::new(read)),
        })
    })?;
    ws_table.set("connect", connect_fn)?;

    let send_fn = lua.create_async_function(|_, (conn, msg): (Value, String)| async move {
        let (sink, _stream) = extract_ws_conn(&conn, "ws.send")?;
        sink.lock()
            .await
            .send(tokio_tungstenite::tungstenite::Message::Text(msg.into()))
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.send: {e}")))?;
        Ok(())
    })?;
    ws_table.set("send", send_fn)?;

    let recv_fn = lua.create_async_function(|_, conn: Value| async move {
        let (_sink, stream) = extract_ws_conn(&conn, "ws.recv")?;
        loop {
            let msg = stream
                .lock()
                .await
                .next()
                .await
                .ok_or_else(|| mlua::Error::runtime("ws.recv: connection closed"))?
                .map_err(|e| mlua::Error::runtime(format!("ws.recv: {e}")))?;
            match msg {
                tokio_tungstenite::tungstenite::Message::Text(t) => {
                    return Ok(t.to_string());
                }
                tokio_tungstenite::tungstenite::Message::Binary(b) => {
                    return String::from_utf8(b.into())
                        .map_err(|e| mlua::Error::runtime(format!("ws.recv: invalid UTF-8: {e}")));
                }
                tokio_tungstenite::tungstenite::Message::Close(_) => {
                    return Err(mlua::Error::runtime("ws.recv: connection closed"));
                }
                _ => continue,
            }
        }
    })?;
    ws_table.set("recv", recv_fn)?;

    let close_fn = lua.create_async_function(|_, conn: Value| async move {
        let (sink, _stream) = extract_ws_conn(&conn, "ws.close")?;
        sink.lock()
            .await
            .close()
            .await
            .map_err(|e| mlua::Error::runtime(format!("ws.close: {e}")))?;
        Ok(())
    })?;
    ws_table.set("close", close_fn)?;

    lua.globals().set("ws", ws_table)?;
    Ok(())
}
