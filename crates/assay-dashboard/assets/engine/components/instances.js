/* Engine console — Instances pane.
 *
 * SQLite engines run single-instance so this typically lists one row;
 * PG engines list every live process keyed by `engine.instances.id`.
 * Stale rows are pruned by the boot loop's heartbeat task.
 */

var AssayEngineInstances = (function () {
  'use strict';

  let ctx = null;

  function render(el, c) {
    ctx = c;
    el.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Instances</h2>' +
        '<button type="button" class="btn btn-small" id="instances-refresh">Refresh</button>' +
      '</div>' +
      '<div id="engine-instances-wrap"><div class="auth-empty">Loading…</div></div>';
    document.getElementById('instances-refresh').addEventListener('click', load);
    load();
  }

  async function load() {
    const wrap = document.getElementById('engine-instances-wrap');
    try {
      const data = await ctx.api.listInstances();
      paint(wrap, data.items || []);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function paint(wrap, items) {
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No live instances registered.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>ID</th><th>Started</th><th>Last heartbeat</th><th>Version</th><th>Modules</th>' +
      '</tr></thead><tbody>';
    for (let i = 0; i < items.length; i++) {
      const r = items[i];
      html += '<tr>' +
        '<td class="auth-mono">' + ctx.escapeHtml(r.id) + '</td>' +
        '<td>' + ctx.escapeHtml(ctx.formatTime(r.started_at)) + '</td>' +
        '<td>' + ctx.escapeHtml(ctx.formatRelative(r.last_heartbeat)) + '</td>' +
        '<td>' + ctx.escapeHtml(r.version || '-') + '</td>' +
        '<td>' + (r.namespaces || []).map(function (n) {
          return '<code>' + ctx.escapeHtml(n) + '</code>';
        }).join(' ') + '</td>' +
      '</tr>';
    }
    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  if (typeof window !== 'undefined') {
    window.AssayEngineInstances = { render: render };
  }
  return { render: render };
})();
