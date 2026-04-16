/* Assay Workflow Dashboard - Workers Component */

var AssayWorkers = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let refreshTimer = null;

  function render(el, context) {
    ctx = context;
    container = el;

    // Clear any previous timer
    if (refreshTimer) clearInterval(refreshTimer);

    el.innerHTML =
      '<h2 class="section-title">Workers</h2>' +
      '<div id="workers-table-wrap"></div>';

    loadWorkers();

    // Auto-refresh every 10 seconds
    refreshTimer = setInterval(loadWorkers, 10000);

    // Clean up on view switch by watching for container removal
    var observer = new MutationObserver(function () {
      if (!document.body.contains(el)) {
        clearInterval(refreshTimer);
        refreshTimer = null;
        observer.disconnect();
      }
    });
    observer.observe(document.body, { childList: true, subtree: true });
  }

  async function loadWorkers() {
    var wrap = container.querySelector('#workers-table-wrap');
    if (!wrap) return;

    try {
      var workers = await ctx.apiFetch('/workers');
      renderTable(wrap, workers || []);
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, workers) {
    if (workers.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No workers registered</p></div>';
      return;
    }

    var now = Date.now() / 1000;

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Status</th>' +
        '<th>ID</th>' +
        '<th>Identity</th>' +
        '<th>Queue</th>' +
        '<th>Active Tasks</th>' +
        '<th>Last Heartbeat</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < workers.length; i++) {
      var w = workers[i];
      var hbAge = now - (w.last_heartbeat || 0);
      var dotClass = hbAge < 30 ? 'healthy' : hbAge < 60 ? 'warning' : 'stale';
      var maxTasks = w.max_concurrent_workflows || w.max_concurrent_activities || '-';

      html +=
        '<tr>' +
          '<td><span class="worker-dot ' + dotClass + '" title="' +
            (dotClass === 'healthy' ? 'Healthy' : dotClass === 'warning' ? 'Slow heartbeat' : 'Stale') +
          '"></span></td>' +
          '<td class="mono">' + ctx.escapeHtml(ctx.truncate(w.id, 24)) + '</td>' +
          '<td>' + ctx.escapeHtml(w.identity || '-') + '</td>' +
          '<td class="mono">' + ctx.escapeHtml(w.task_queue || '-') + '</td>' +
          '<td>' + (w.active_tasks || 0) + '/' + maxTasks + '</td>' +
          '<td>' + ctx.formatTime(w.last_heartbeat) + '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  return { render: render };
})();
