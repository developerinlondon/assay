(function () {
  var statusEl = document.getElementById("shell-status");
  function setStatus(text, cls) {
    if (!statusEl) return;
    statusEl.textContent = text;
    statusEl.className = "pill " + cls;
  }

  var term = new Terminal({
    cursorBlink: true,
    fontFamily: "JetBrains Mono, ui-monospace, monospace",
    fontSize: 13,
    scrollback: 5000,
    theme: {
      background: "#000000",
      foreground: "#cbd1dc",
      cursor: "#5ad6c2",
      selectionBackground: "#1b2029",
    },
  });

  var fit = new FitAddon.FitAddon();
  term.loadAddon(fit);
  term.open(document.getElementById("terminal"));
  fit.fit();

  var url = (location.protocol === "https:" ? "wss://" : "ws://") + location.host + window.SHELL_WS_URL;
  var ws = new WebSocket(url);
  ws.binaryType = "arraybuffer";

  function sendResize() {
    if (ws.readyState !== WebSocket.OPEN) return;
    ws.send(JSON.stringify({ resize: { cols: term.cols, rows: term.rows } }));
  }

  ws.onopen = function () {
    setStatus("connected", "pill-ok");
    sendResize();
    term.focus();
  };

  ws.onmessage = function (ev) {
    if (typeof ev.data === "string") {
      term.write(ev.data);
    } else {
      term.write(new Uint8Array(ev.data));
    }
  };

  ws.onclose = function () { setStatus("disconnected", "pill-muted"); };
  ws.onerror = function () { setStatus("error", "pill-err"); };

  term.onData(function (data) {
    if (ws.readyState === WebSocket.OPEN) ws.send(data);
  });

  var resizeTimer;
  window.addEventListener("resize", function () {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(function () { fit.fit(); sendResize(); }, 80);
  });
})();
