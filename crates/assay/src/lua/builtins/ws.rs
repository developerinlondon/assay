use futures_util::{SinkExt, StreamExt};
use mlua::{Lua, Table, UserData, UserDataFields, UserDataMethods, Value};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio_tungstenite::Connector;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{HeaderName, HeaderValue};
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
    protocol: Option<String>,
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

// TLS certificate verifier that accepts any server certificate. Wired in only
// when `ws.connect` is called with `opts.insecure = true`.
#[derive(Debug)]
struct NoCertVerifier {
    provider: Arc<rustls::crypto::CryptoProvider>,
}

impl rustls::client::danger::ServerCertVerifier for NoCertVerifier {
    fn verify_server_cert(
        &self,
        _end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        Ok(rustls::client::danger::ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

// Returns `None` for the default (certificate-verified) path so
// tokio-tungstenite uses its built-in webpki roots. With `insecure`, returns a
// connector that skips certificate verification.
fn build_ws_connector(insecure: bool) -> mlua::Result<Option<Connector>> {
    if !insecure {
        return Ok(None);
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| mlua::Error::runtime(format!("ws.connect: TLS setup failed: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(NoCertVerifier { provider }))
        .with_no_client_auth();
    Ok(Some(Connector::Rustls(Arc::new(config))))
}

pub fn register_ws(lua: &Lua) -> mlua::Result<()> {
    let ws_table = lua.create_table()?;

    let connect_fn =
        lua.create_async_function(|lua, (url, opts): (String, Option<Table>)| async move {
            let mut subprotocols: Vec<String> = Vec::new();
            let mut header_pairs: Vec<(String, String)> = Vec::new();
            let mut insecure = false;
            if let Some(opts) = &opts {
                if let Ok(list) = opts.get::<Table>("subprotocols") {
                    for item in list.sequence_values::<String>() {
                        subprotocols.push(item?);
                    }
                }
                if let Ok(headers) = opts.get::<Table>("headers") {
                    for pair in headers.pairs::<String, String>() {
                        let (name, value) = pair?;
                        header_pairs.push((name, value));
                    }
                }
                insecure = opts.get::<Option<bool>>("insecure")?.unwrap_or(false);
            }

            let mut request = url
                .as_str()
                .into_client_request()
                .map_err(|e| mlua::Error::runtime(format!("ws.connect: {e}")))?;
            {
                let headers = request.headers_mut();
                if !subprotocols.is_empty() {
                    let joined = subprotocols.join(", ");
                    let value = HeaderValue::from_str(&joined).map_err(|e| {
                        mlua::Error::runtime(format!("ws.connect: invalid subprotocol: {e}"))
                    })?;
                    headers.insert("Sec-WebSocket-Protocol", value);
                }
                for (name, value) in header_pairs {
                    let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|e| {
                        mlua::Error::runtime(format!(
                            "ws.connect: invalid header name '{name}': {e}"
                        ))
                    })?;
                    let header_value = HeaderValue::from_str(&value).map_err(|e| {
                        mlua::Error::runtime(format!(
                            "ws.connect: invalid header value for '{name}': {e}"
                        ))
                    })?;
                    headers.append(header_name, header_value);
                }
            }

            let connector = build_ws_connector(insecure)?;
            let (stream, response) =
                tokio_tungstenite::connect_async_tls_with_config(request, None, false, connector)
                    .await
                    .map_err(|e| mlua::Error::runtime(format!("ws.connect: {e}")))?;

            let protocol = response
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());

            let (sink, read) = stream.split();
            lua.create_any_userdata(WsConn {
                sink: Rc::new(tokio::sync::Mutex::new(sink)),
                stream: Rc::new(tokio::sync::Mutex::new(read)),
                protocol,
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

    let send_binary_fn =
        lua.create_async_function(|_, (conn, data): (Value, mlua::String)| async move {
            let (sink, _stream) = extract_ws_conn(&conn, "ws.send_binary")?;
            let bytes = data.as_bytes().to_vec();
            sink.lock()
                .await
                .send(Message::Binary(bytes.into()))
                .await
                .map_err(|e| mlua::Error::runtime(format!("ws.send_binary: {e}")))?;
            Ok(())
        })?;
    ws_table.set("send_binary", send_binary_fn)?;

    // Returns raw frame bytes as a (binary-safe) Lua string. Text frames come
    // back as their UTF-8 bytes; binary frames come back verbatim, so callers
    // that need non-UTF-8 payloads (e.g. Kubernetes exec channels) get the raw
    // bytes rather than a decode error.
    let recv_fn = lua.create_async_function(|lua, conn: Value| async move {
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
                Message::Text(t) => {
                    return Ok(Value::String(lua.create_string(t.as_bytes())?));
                }
                Message::Binary(b) => {
                    return Ok(Value::String(lua.create_string(&b)?));
                }
                Message::Close(_) => {
                    return Err(mlua::Error::runtime("ws.recv: connection closed"));
                }
                _ => continue,
            }
        }
    })?;
    ws_table.set("recv", recv_fn)?;

    let protocol_fn = lua.create_function(|lua, conn: Value| {
        let ud = match &conn {
            Value::UserData(ud) => ud,
            _ => {
                return Err(mlua::Error::runtime(
                    "ws.protocol: first argument must be a ws connection",
                ));
            }
        };
        let ws = ud.borrow::<WsConn>().map_err(|_| {
            mlua::Error::runtime("ws.protocol: first argument must be a ws connection")
        })?;
        match &ws.protocol {
            Some(p) => Ok(Value::String(lua.create_string(p)?)),
            None => Ok(Value::Nil),
        }
    })?;
    ws_table.set("protocol", protocol_fn)?;

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
