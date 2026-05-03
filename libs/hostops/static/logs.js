(function () {
  "use strict";

  var es        = null;
  var paused    = false;
  var priority  = "";
  var lineTimes = [];
  var reconnectDelay = 1;

  function buildUrl() {
    var params = [];
    var machineEl = document.getElementById("log-machine");
    var machine = machineEl ? machineEl.value : (window.KNOWHERE_LOG_MACHINE || "");
    var unitEl  = document.getElementById("log-unit");
    var unit    = unitEl ? unitEl.value.trim() : "";
    if (machine)  params.push("machine=" + encodeURIComponent(machine));
    if (unit)     params.push("unit="    + encodeURIComponent(unit));
    if (priority !== "") params.push("priority=" + encodeURIComponent(priority));
    return "/api/logs/stream" + (params.length ? "?" + params.join("&") : "");
  }

  function priorityClass(p) {
    if (p == null) return "";
    if (p <= 3) return " err";
    if (p === 4) return " warn";
    return "";
  }

  function fmtTs(ts) {
    if (!ts) return "";
    var d = new Date(Math.floor(ts / 1000));
    var h = String(d.getHours()).padStart(2, "0");
    var m = String(d.getMinutes()).padStart(2, "0");
    var s = String(d.getSeconds()).padStart(2, "0");
    var ms = String(d.getMilliseconds()).padStart(3, "0");
    return h + ":" + m + ":" + s + "." + ms;
  }

  function isNearBottom(vp) {
    return vp.scrollHeight - vp.scrollTop - vp.clientHeight < 80;
  }

  function appendEntry(entry) {
    var vp = document.getElementById("log-viewport");
    if (!vp) return;

    var search = document.getElementById("log-search").value.toLowerCase();
    var msg    = (entry.message || "").toLowerCase();
    if (search && msg.indexOf(search) === -1) return;

    var wasBottom = isNearBottom(vp);

    var div  = document.createElement("div");
    div.className = "ln" + priorityClass(entry.priority);

    var ts   = document.createElement("span");
    ts.className = "ts";
    ts.textContent = fmtTs(entry.ts);

    var src  = document.createElement("span");
    src.className = "src";
    src.textContent = entry.unit || entry.hostname || "";

    var msgEl = document.createElement("span");
    msgEl.className = "msg";
    msgEl.textContent = entry.message || "";

    div.appendChild(ts);
    div.appendChild(src);
    div.appendChild(msgEl);
    vp.appendChild(div);

    if (wasBottom) vp.scrollTop = vp.scrollHeight;

    lineTimes.push(Date.now());
  }

  function updateRate() {
    var now = Date.now();
    lineTimes = lineTimes.filter(function (t) { return now - t < 5000; });
    var rate  = (lineTimes.length / 5).toFixed(1);
    var el    = document.getElementById("line-rate");
    if (el) el.textContent = rate + " ln/s";
  }

  function setStatus(text) {
    var el = document.getElementById("stream-status");
    if (el) el.textContent = text;
  }

  function connect() {
    if (paused) return;
    if (es) { es.close(); es = null; }

    setStatus("connecting");
    es = new EventSource(buildUrl());

    es.addEventListener("log", function (e) {
      reconnectDelay = 1;
      try {
        var entry = JSON.parse(e.data);
        appendEntry(entry);
      } catch (_) {}
    });

    es.onopen = function () {
      setStatus("following");
    };

    es.onerror = function () {
      setStatus("reconnecting in " + reconnectDelay + "s");
      es.close();
      es = null;
      setTimeout(function () {
        reconnectDelay = Math.min(reconnectDelay * 2, 8);
        connect();
      }, reconnectDelay * 1000);
    };
  }

  function reconnect() {
    reconnectDelay = 1;
    connect();
  }

  function knowhereLogsInit() {
    var pauseBtn = document.getElementById("log-pause");
    var clearBtn = document.getElementById("log-clear");

    if (pauseBtn) {
      pauseBtn.addEventListener("click", function () {
        paused = !paused;
        if (paused) {
          if (es) { es.close(); es = null; }
          pauseBtn.textContent = "Resume";
          setStatus("paused");
        } else {
          pauseBtn.textContent = "Pause";
          reconnect();
        }
      });
    }

    if (clearBtn) {
      clearBtn.addEventListener("click", function () {
        var vp = document.getElementById("log-viewport");
        if (vp) vp.innerHTML = "";
        lineTimes = [];
      });
    }

    document.querySelectorAll(".chip[data-priority]").forEach(function (chip) {
      chip.addEventListener("click", function () {
        document.querySelectorAll(".chip[data-priority]").forEach(function (c) {
          c.classList.remove("active");
        });
        chip.classList.add("active");
        priority = chip.getAttribute("data-priority");
        reconnect();
      });
    });

    var debounceTimer = null;
    ["log-machine", "log-unit"].forEach(function (id) {
      var el = document.getElementById(id);
      if (el) {
        el.addEventListener("change", function () {
          clearTimeout(debounceTimer);
          debounceTimer = setTimeout(reconnect, 300);
        });
      }
    });

    var searchEl = document.getElementById("log-search");
    if (searchEl) {
      searchEl.addEventListener("input", function () {
        // search filters client-side, no reconnect needed
      });
    }

    setInterval(updateRate, 1000);

    connect();
  }

  window.knowhereLogsInit = knowhereLogsInit;
})();
