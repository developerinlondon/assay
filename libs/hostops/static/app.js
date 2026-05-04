const evt = new EventSource("/api/events");
evt.addEventListener("refresh", () => {
  document.body.dispatchEvent(new CustomEvent("dashboard-refresh"));
});
evt.onerror = () => console.warn("knowhere SSE: stream error, browser will auto-reconnect");

// v0.2.0-beta.1 — collapsible sidebar state persistence (plan 09 task 15)
(function () {
  document.querySelectorAll('details[data-section]').forEach(function (el) {
    var key = 'sidebar.' + el.dataset.section;
    var saved = localStorage.getItem(key);
    if (saved === 'open') el.open = true;
    if (saved === 'closed') el.open = false;
    el.addEventListener('toggle', function () {
      localStorage.setItem(key, el.open ? 'open' : 'closed');
    });
  });
})();

// Sidebar mutex highlight — one entry highlighted at a time. Clicking a
// group summary (which doesn't navigate) moves the highlight onto the
// summary; clicking any other sidebar link clears it so the post-nav
// render of .active wins. Reload always re-syncs to the URL.
(function () {
  document.querySelectorAll('details[data-section] > summary').forEach(function (s) {
    s.addEventListener('click', function () {
      document.querySelectorAll('aside.sidebar a.active').forEach(function (a) {
        a.classList.remove('active');
      });
      document.querySelectorAll('summary.summary-selected').forEach(function (other) {
        if (other !== s) other.classList.remove('summary-selected');
      });
      s.classList.add('summary-selected');
    });
  });
  document.querySelectorAll('aside.sidebar a').forEach(function (a) {
    a.addEventListener('click', function () {
      document.querySelectorAll('summary.summary-selected').forEach(function (s) {
        s.classList.remove('summary-selected');
      });
    });
  });
})();
