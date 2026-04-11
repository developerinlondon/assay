## ws

WebSocket client. No `require()` needed.

- `ws.connect(url)` → conn — Connect to WebSocket server
- `ws.send(conn, msg)` → nil — Send message
- `ws.recv(conn)` → string — Receive message (blocking)
- `ws.close(conn)` → nil — Close connection
