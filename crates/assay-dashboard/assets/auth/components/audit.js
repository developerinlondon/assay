/* Audit log pane — paginated viewer with actor/action/time filters.
 *
 * The auth.audit table is currently deferred (V1 schema notes), so the
 * server returns an empty list with `enabled: false`. This pane renders
 * the filter shape today so the UI is ready when the table lands. */

var AssayAuthAudit = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let state = { actor: '', action: '', since: '', until: '', limit: 50, offset: 0 };

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Audit Log</h2>' +
      '</div>' +
      '<div class="auth-form">' +
        '<label for="al-actor">Actor</label><input type="text" id="al-actor" value="' + ctx.escapeHtml(state.actor) + '" />' +
        '<label for="al-action">Action</label><input type="text" id="al-action" value="' + ctx.escapeHtml(state.action) + '" />' +
        '<label for="al-since">Since (epoch s)</label><input type="number" id="al-since" value="' + ctx.escapeHtml(state.since) + '" />' +
        '<label for="al-until">Until (epoch s)</label><input type="number" id="al-until" value="' + ctx.escapeHtml(state.until) + '" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="al-load">Load</button>' +
        '</div>' +
      '</div>' +
      '<div id="al-wrap" style="margin-top:16px;"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('al-load').addEventListener('click', function () {
      state.actor = document.getElementById('al-actor').value.trim();
      state.action = document.getElementById('al-action').value.trim();
      state.since = document.getElementById('al-since').value.trim();
      state.until = document.getElementById('al-until').value.trim();
      state.offset = 0;
      load();
    });

    load();
  }

  async function load() {
    const wrap = container.querySelector('#al-wrap');
    try {
      const data = await ctx.api.audit({
        limit: state.limit,
        offset: state.offset,
        actor: state.actor || undefined,
        action: state.action || undefined,
        since: state.since || undefined,
        until: state.until || undefined,
      });
      if (data && data.enabled === false) {
        wrap.innerHTML = '<div class="auth-empty">' +
          '<p>The <code>auth.audit</code> table is materialised but no rows are written yet.</p>' +
          '<p>Audit emission lands in a follow-up phase; this pane will populate automatically once it does.</p>' +
        '</div>';
        return;
      }
      const items = (data && data.items) || [];
      if (!items.length) {
        wrap.innerHTML = '<div class="auth-empty">No audit rows match.</div>';
        return;
      }
      let html = '<table class="data-table"><thead><tr><th>Time</th><th>Actor</th><th>Action</th><th>Detail</th></tr></thead><tbody>';
      items.forEach(function (r) {
        html += '<tr>' +
          '<td>' + ctx.formatTime(r.created_at) + '</td>' +
          '<td>' + ctx.escapeHtml(r.actor || '-') + '</td>' +
          '<td>' + ctx.escapeHtml(r.action || '-') + '</td>' +
          '<td class="auth-mono">' + ctx.escapeHtml(JSON.stringify(r.detail || {})) + '</td>' +
        '</tr>';
      });
      html += '</tbody></table>';
      wrap.innerHTML = html;
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthAudit = { render: render };
  }

  return { render: render };
})();
