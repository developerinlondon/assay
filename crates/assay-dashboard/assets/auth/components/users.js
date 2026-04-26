/* Users pane — list / create / view / update / delete / reset password. */

var AssayAuthUsers = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let state = { search: '', limit: 50, offset: 0 };

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Users</h2>' +
        '<input type="search" class="auth-search" id="users-search" placeholder="Search by email/name…" value="' + ctx.escapeHtml(state.search) + '" />' +
        '<button type="button" class="btn btn-primary" id="users-new">New user</button>' +
      '</div>' +
      '<div id="users-table-wrap"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('users-search').addEventListener('input', function (e) {
      state.search = e.target.value;
      state.offset = 0;
      load();
    });
    document.getElementById('users-new').addEventListener('click', openCreate);

    container.addEventListener('click', function (e) {
      const row = e.target.closest('tr[data-user-id]');
      if (!row) return;
      const action = e.target.closest('[data-action]');
      const id = row.dataset.userId;
      if (action) {
        e.stopPropagation();
        const a = action.dataset.action;
        if (a === 'delete') return doDelete(id);
        if (a === 'reset') return openResetPassword(id);
        if (a === 'view') return showDetail(id);
      } else {
        showDetail(id);
      }
    });

    load();
  }

  async function load() {
    const wrap = container.querySelector('#users-table-wrap');
    try {
      const data = await ctx.api.listUsers({
        limit: state.limit,
        offset: state.offset,
        search: state.search || undefined,
      });
      renderTable(wrap, data);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderTable(wrap, data) {
    const items = (data && data.items) || [];
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No users.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>ID</th><th>Email</th><th>Display name</th><th>Verified</th><th>Created</th><th></th>' +
      '</tr></thead><tbody>';
    for (let i = 0; i < items.length; i++) {
      const u = items[i];
      html += '<tr class="clickable-row" data-user-id="' + ctx.escapeHtml(u.id) + '">' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(u.id, 22)) + '</td>' +
        '<td>' + ctx.escapeHtml(u.email || '-') + '</td>' +
        '<td>' + ctx.escapeHtml(u.display_name || '-') + '</td>' +
        '<td>' + (u.email_verified ? '✓' : '—') + '</td>' +
        '<td>' + ctx.formatTime(u.created_at) + '</td>' +
        '<td>' +
          '<button class="btn btn-small" data-action="reset">Reset password</button> ' +
          '<button class="btn btn-small btn-danger" data-action="delete">Delete</button>' +
        '</td>' +
      '</tr>';
    }
    html += '</tbody></table>';
    if (data.total > items.length) {
      html += '<div class="auth-toolbar" style="margin-top:12px;">' +
        '<span>' + (state.offset + 1) + '-' + (state.offset + items.length) + ' of ' + data.total + '</span>' +
        '<button class="btn btn-small" id="users-prev"' + (state.offset === 0 ? ' disabled' : '') + '>Prev</button>' +
        '<button class="btn btn-small" id="users-next"' + (state.offset + items.length >= data.total ? ' disabled' : '') + '>Next</button>' +
      '</div>';
    }
    wrap.innerHTML = html;
    const prev = document.getElementById('users-prev');
    const next = document.getElementById('users-next');
    if (prev) prev.addEventListener('click', function () {
      state.offset = Math.max(0, state.offset - state.limit); load();
    });
    if (next) next.addEventListener('click', function () {
      state.offset += state.limit; load();
    });
  }

  function openCreate() {
    const wrap = container.querySelector('#users-table-wrap');
    wrap.innerHTML = '<h3>New user</h3>' +
      '<div class="auth-form">' +
        '<label for="nu-email">Email</label><input type="email" id="nu-email" />' +
        '<label for="nu-name">Display name</label><input type="text" id="nu-name" />' +
        '<label for="nu-verified">Email verified</label><input type="checkbox" id="nu-verified" />' +
        '<label for="nu-pw">Password (optional)</label><input type="password" id="nu-pw" autocomplete="new-password" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="nu-create">Create</button>' +
          '<button type="button" class="btn" id="nu-cancel">Cancel</button>' +
        '</div>' +
      '</div>';
    document.getElementById('nu-cancel').addEventListener('click', load);
    document.getElementById('nu-create').addEventListener('click', async function () {
      const body = {
        email: document.getElementById('nu-email').value || null,
        display_name: document.getElementById('nu-name').value || null,
        email_verified: document.getElementById('nu-verified').checked,
        password: document.getElementById('nu-pw').value || null,
      };
      try {
        await ctx.api.createUser(body);
        ctx.toast('User created', 'info');
        load();
      } catch (err) {
        ctx.toast('Create failed: ' + err.message, 'error');
      }
    });
  }

  async function showDetail(id) {
    const wrap = container.querySelector('#users-table-wrap');
    wrap.innerHTML = '<div class="auth-empty">Loading user detail…</div>';
    try {
      const d = await ctx.api.getUser(id);
      let html = '<button class="btn btn-small" id="users-back">&larr; Back</button>' +
        '<h3>' + ctx.escapeHtml(d.user.email || d.user.id) + '</h3>' +
        '<div class="auth-form">' +
          '<label for="ud-email">Email</label><input type="email" id="ud-email" value="' + ctx.escapeHtml(d.user.email || '') + '" />' +
          '<label for="ud-name">Display name</label><input type="text" id="ud-name" value="' + ctx.escapeHtml(d.user.display_name || '') + '" />' +
          '<label for="ud-verified">Email verified</label><input type="checkbox" id="ud-verified"' + (d.user.email_verified ? ' checked' : '') + ' />' +
          '<div class="auth-form-actions">' +
            '<button type="button" class="btn btn-primary" id="ud-save">Save</button>' +
            '<button type="button" class="btn btn-danger" id="ud-delete">Delete</button>' +
          '</div>' +
        '</div>' +
        '<div class="auth-pane-section"><h3>Passkeys (' + d.passkeys.length + ')</h3>';
      if (!d.passkeys.length) {
        html += '<p class="auth-empty">No passkeys registered.</p>';
      } else {
        html += '<table class="data-table"><thead><tr><th>Credential ID</th><th>Sign count</th><th>Transports</th><th>Created</th></tr></thead><tbody>';
        d.passkeys.forEach(function (p) {
          html += '<tr><td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(p.credential_id, 32)) +
            '</td><td>' + p.sign_count + '</td><td>' + ctx.escapeHtml((p.transports || []).join(', ')) +
            '</td><td>' + ctx.formatTime(p.created_at) + '</td></tr>';
        });
        html += '</tbody></table>';
      }
      html += '</div>';

      html += '<div class="auth-pane-section"><h3>Sessions (' + d.sessions.length + ')</h3>';
      if (!d.sessions.length) {
        html += '<p class="auth-empty">No active sessions.</p>';
      } else {
        html += '<table class="data-table"><thead><tr><th>Session ID</th><th>Created</th><th>Expires</th><th></th></tr></thead><tbody>';
        d.sessions.forEach(function (s) {
          html += '<tr><td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(s.id, 28)) +
            '</td><td>' + ctx.formatTime(s.created_at) + '</td>' +
            '<td>' + ctx.formatTime(s.expires_at) + '</td>' +
            '<td><button class="btn btn-small" data-revoke="' + ctx.escapeHtml(s.id) + '">Revoke</button></td></tr>';
        });
        html += '</tbody></table>';
        html += '<button class="btn btn-small btn-danger" id="ud-revoke-all" style="margin-top:8px;">Revoke all sessions</button>';
      }
      html += '</div>';

      html += '<div class="auth-pane-section"><h3>Federated identities (' + d.upstream.length + ')</h3>';
      if (!d.upstream.length) {
        html += '<p class="auth-empty">No upstream links.</p>';
      } else {
        html += '<table class="data-table"><thead><tr><th>Provider</th><th>Subject</th></tr></thead><tbody>';
        d.upstream.forEach(function (u) {
          html += '<tr><td>' + ctx.escapeHtml(u.provider) + '</td><td class="auth-mono">' + ctx.escapeHtml(u.subject) + '</td></tr>';
        });
        html += '</tbody></table>';
      }
      html += '</div>';

      wrap.innerHTML = html;
      document.getElementById('users-back').addEventListener('click', load);
      document.getElementById('ud-save').addEventListener('click', async function () {
        try {
          await ctx.api.updateUser(id, {
            email: document.getElementById('ud-email').value || null,
            display_name: document.getElementById('ud-name').value || null,
            email_verified: document.getElementById('ud-verified').checked,
          });
          ctx.toast('Saved', 'info');
          showDetail(id);
        } catch (err) { ctx.toast('Save failed: ' + err.message, 'error'); }
      });
      document.getElementById('ud-delete').addEventListener('click', function () { doDelete(id); });
      const revokeAll = document.getElementById('ud-revoke-all');
      if (revokeAll) revokeAll.addEventListener('click', async function () {
        if (!confirm('Revoke ALL sessions for this user?')) return;
        try {
          const r = await ctx.api.revokeAllSessions(id);
          ctx.toast('Revoked ' + (r.revoked || 0) + ' sessions', 'info');
          showDetail(id);
        } catch (err) { ctx.toast('Revoke failed: ' + err.message, 'error'); }
      });
      wrap.querySelectorAll('button[data-revoke]').forEach(function (btn) {
        btn.addEventListener('click', async function () {
          const sid = btn.dataset.revoke;
          try {
            await ctx.api.revokeSession(sid);
            ctx.toast('Session revoked', 'info');
            showDetail(id);
          } catch (err) { ctx.toast('Revoke failed: ' + err.message, 'error'); }
        });
      });
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  async function doDelete(id) {
    if (!confirm('Permanently delete user ' + id + '? Sessions, passkeys, and upstream links cascade.')) return;
    try {
      await ctx.api.deleteUser(id);
      ctx.toast('Deleted', 'info');
      load();
    } catch (err) { ctx.toast('Delete failed: ' + err.message, 'error'); }
  }

  function openResetPassword(id) {
    const pw = prompt('New password for ' + id + ':');
    if (!pw) return;
    ctx.api.resetPassword(id, pw)
      .then(function () { ctx.toast('Password reset', 'info'); })
      .catch(function (err) { ctx.toast('Reset failed: ' + err.message, 'error'); });
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthUsers = { render: render };
  }

  return { render: render };
})();
