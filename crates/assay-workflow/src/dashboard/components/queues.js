/* Assay Workflow Dashboard - Queues Component */

var AssayQueues = (function () {
  'use strict';

  let ctx = null;
  let container = null;

  function render(el, context) {
    ctx = context;
    container = el;

    el.innerHTML =
      '<h2 class="section-title">Queues</h2>' +
      '<div id="queues-table-wrap"></div>';

    loadQueues();
  }

  async function loadQueues() {
    var wrap = container.querySelector('#queues-table-wrap');
    try {
      var queues = await ctx.apiFetch('/queues');
      renderTable(wrap, queues || []);
    } catch (err) {
      wrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderTable(wrap, queues) {
    if (queues.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No queues found</p></div>';
      return;
    }

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Queue</th>' +
        '<th>Pending</th>' +
        '<th>Running</th>' +
        '<th>Workers</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < queues.length; i++) {
      var q = queues[i];
      var warn = (q.pending_activities > 0 && q.workers === 0);

      html +=
        '<tr>' +
          '<td class="mono">' + ctx.escapeHtml(q.queue || q.name || '-') + '</td>' +
          '<td>' + (q.pending_activities || 0) +
            (warn ? ' <span class="warning-icon" title="No workers available">&#9888;</span>' : '') +
          '</td>' +
          '<td>' + (q.running_activities || 0) + '</td>' +
          '<td>' + (q.workers || 0) + '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  return { render: render };
})();
