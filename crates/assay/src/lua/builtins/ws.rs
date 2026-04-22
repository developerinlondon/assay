use futures_util::{SinkExt, StreamExt};
use mlua::{Lua, UserData, Value};
use std::rc::Rc;
use tokio_tungstenite::MaybeTlsStream;

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
