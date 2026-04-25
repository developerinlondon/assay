/* Sessions pane — global list with optional user filter, revoke single + revoke all. */

var AssayAuthSessions = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let state = { user_id: '', limit: 50, offset: 0 };

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Sessions</h2>' +
        '<input type="search" class="auth-search" id="ses-user" placeholder="Filter by user_id…" value="' + ctx.escapeHtml(state.user_id) + '" />' +
        '<button type="button" class="btn" id="ses-refresh">Refresh</button>' +
      '</div>' +
      '<div id="ses-table-wrap"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('ses-user').addEventListener('input', function (e) {
      state.user_id = e.target.value.trim();
      state.offset = 0;
      load();
    });
    document.getElementById('ses-refresh').addEventListener('click', load);

    container.addEventListener('click', function (e) {
      const btn = e.target.closest('[data-action]');
      if (!btn) return;
      const a = btn.dataset.action;
      if (a === 'revoke') return revoke(btn.dataset.id);
      if (a === 'revoke-all') return revokeAll(btn.dataset.user);
    });

    load();
  }

  async function load() {
    const wrap = container.querySelector('#ses-table-wrap');
    try {
      const data = await ctx.api.listSessions({
        user_id: state.user_id || undefined,
        limit: state.limit,
        offset: state.offset,
      });
      renderTable(wrap, data);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderTable(wrap, data) {
    const items = (data && data.items) || [];
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No sessions.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>Session ID</th><th>User</th><th>Created</th><th>Expires</th><th></th>' +
      '</tr></thead><tbody>';
    for (let i = 0; i < items.length; i++) {
      const s = items[i];
      html += '<tr>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(s.id, 28)) + '</td>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(s.user_id, 22)) + '</td>' +
        '<td>' + ctx.formatTime(s.created_at) + '</td>' +
        '<td>' + ctx.formatTime(s.expires_at) + '</td>' +
        '<td>' +
          '<button class="btn btn-small" data-action="revoke" data-id="' + ctx.escapeHtml(s.id) + '">Revoke</button> ' +
          '<button class="btn btn-small" data-action="revoke-all" data-user="' + ctx.escapeHtml(s.user_id) + '">Revoke all for user</button>' +
        '</td>' +
      '</tr>';
    }
    html += '</tbody></table>';
    if (data.total > items.length) {
      html += '<div class="auth-toolbar" style="margin-top:12px;">' +
        '<span>' + (state.offset + 1) + '-' + (state.offset + items.length) + ' of ' + data.total + '</span>' +
        '<button class="btn btn-small" id="ses-prev"' + (state.offset === 0 ? ' disabled' : '') + '>Prev</button>' +
        '<button class="btn btn-small" id="ses-next"' + (state.offset + items.length >= data.total ? ' disabled' : '') + '>Next</button>' +
      '</div>';
    }
    wrap.innerHTML = html;
    const prev = document.getElementById('ses-prev');
    const next = document.getElementById('ses-next');
    if (prev) prev.addEventListener('click', function () {
      state.offset = Math.max(0, state.offset - state.limit); load();
    });
    if (next) next.addEventListener('click', function () {
      state.offset += state.limit; load();
    });
  }

  async function revoke(id) {
    if (!confirm('Revoke session ' + id + '?')) return;
    try {
      await ctx.api.revokeSession(id);
      ctx.toast('Session revoked', 'info');
      load();
    } catch (err) { ctx.toast('Revoke failed: ' + err.message, 'error'); }
  }

  async function revokeAll(userId) {
    if (!confirm('Revoke ALL sessions for user ' + userId + '?')) return;
    try {
      const r = await ctx.api.revokeAllSessions(userId);
      ctx.toast('Revoked ' + (r.revoked || 0) + ' sessions', 'info');
      load();
    } catch (err) { ctx.toast('Revoke failed: ' + err.message, 'error'); }
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthSessions = { render: render };
  }

  return { render: render };
})();
