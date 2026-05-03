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
