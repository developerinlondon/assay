/* Engine console — Audit pane.
 *
 * Paginated viewer over `engine.audit`. Lets operators filter by
 * actor + action and walk by limit/offset. Details cell renders the
 * JSON blob inline (truncated) — full payload viewable on row click.
 */

var AssayEngineAudit = (function () {
  'use strict';

  let ctx = null;
  let state = { limit: 50, offset: 0, actor: '', action: '' };

  function render(el, c) {
    ctx = c;
    el.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Audit</h2>' +
        '<input type="search" class="auth-search" id="audit-actor" placeholder="actor…" value="' + ctx.escapeHtml(state.actor) + '" />' +
        '<input type="search" class="auth-search" id="audit-action" placeholder="action…" value="' + ctx.escapeHtml(state.action) + '" />' +
        '<button type="button" class="btn btn-small" id="audit-refresh">Refresh</button>' +
      '</div>' +
      '<div id="engine-audit-wrap"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('audit-actor').addEventListener('input', function (e) {
      state.actor = e.target.value; state.offset = 0; load();
    });
    document.getElementById('audit-action').addEventListener('input', function (e) {
      state.action = e.target.value; state.offset = 0; load();
    });
    document.getElementById('audit-refresh').addEventListener('click', load);
    load();
  }

  async function load() {
    const wrap = document.getElementById('engine-audit-wrap');
    try {
      const data = await ctx.api.listAudit({
        limit: state.limit,
        offset: state.offset,
        actor: state.actor || undefined,
        action: state.action || undefined,
      });
      paint(wrap, data);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function paint(wrap, data) {
    const items = (data && data.items) || [];
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No audit rows match the filter.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>When</th><th>Actor</th><th>Action</th><th>Details</th>' +
      '</tr></thead><tbody>';
    for (let i = 0; i < items.length; i++) {
      const r = items[i];
      const det = r.details ? JSON.stringify(r.details) : '';
      html += '<tr>' +
        '<td>' + ctx.escapeHtml(ctx.formatRelative(r.ts)) + '</td>' +
        '<td>' + ctx.escapeHtml(r.actor || '-') + '</td>' +
        '<td><code>' + ctx.escapeHtml(r.action) + '</code></td>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(det, 96)) + '</td>' +
      '</tr>';
    }
    html += '</tbody></table>';
    html += '<div class="auth-toolbar" style="margin-top:12px;">' +
      '<span>' + (state.offset + 1) + '-' + (state.offset + items.length) + ' of ' + data.total + '</span>' +
      '<button class="btn btn-small" id="audit-prev"' + (state.offset === 0 ? ' disabled' : '') + '>Prev</button>' +
      '<button class="btn btn-small" id="audit-next"' + (state.offset + items.length >= data.total ? ' disabled' : '') + '>Next</button>' +
    '</div>';
    wrap.innerHTML = html;
    document.getElementById('audit-prev').addEventListener('click', function () {
      state.offset = Math.max(0, state.offset - state.limit); load();
    });
    document.getElementById('audit-next').addEventListener('click', function () {
      state.offset += state.limit; load();
    });
  }

  if (typeof window !== 'undefined') {
    window.AssayEngineAudit = { render: render };
  }
  return { render: render };
})();
