/* OIDC Upstream pane — federated SSO sources (Google, GitHub, etc). */

var AssayAuthOidcUpstream = (function () {
  'use strict';

  let ctx = null;
  let container = null;

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">OIDC Upstream Providers</h2>' +
        '<button type="button" class="btn btn-primary" id="up-new">New / Upsert</button>' +
      '</div>' +
      '<div id="up-wrap"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('up-new').addEventListener('click', function () { openForm(null); });

    container.addEventListener('click', function (e) {
      const action = e.target.closest('[data-action]');
      if (!action) return;
      const a = action.dataset.action;
      const slug = action.dataset.slug;
      if (a === 'edit') return openForm(slug);
      if (a === 'delete') return doDelete(slug);
    });

    load();
  }

  async function load() {
    const wrap = container.querySelector('#up-wrap');
    try {
      const items = await ctx.api.listOidcUpstream();
      renderTable(wrap, items || []);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderTable(wrap, items) {
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No upstream providers configured.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>Slug</th><th>Display name</th><th>Issuer</th><th>Client ID</th><th>Enabled</th><th></th>' +
      '</tr></thead><tbody>';
    items.forEach(function (u) {
      html += '<tr>' +
        '<td class="auth-mono">' + ctx.escapeHtml(u.slug) + '</td>' +
        '<td>' + ctx.escapeHtml(u.display_name) + '</td>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(u.issuer, 32)) + '</td>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(u.client_id, 22)) + '</td>' +
        '<td>' + (u.enabled ? '✓' : '—') + '</td>' +
        '<td>' +
          '<button class="btn btn-small" data-action="edit" data-slug="' + ctx.escapeHtml(u.slug) + '">Edit</button> ' +
          '<button class="btn btn-small btn-danger" data-action="delete" data-slug="' + ctx.escapeHtml(u.slug) + '">Delete</button>' +
        '</td>' +
      '</tr>';
    });
    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  async function openForm(slug) {
    const wrap = container.querySelector('#up-wrap');
    let existing = null;
    if (slug) {
      try { existing = await ctx.api.getOidcUpstream(slug); }
      catch (err) { ctx.toast('Load failed: ' + err.message, 'error'); return; }
    }
    const e = existing || {};
    wrap.innerHTML = '<button class="btn btn-small" id="up-back">&larr; Back</button>' +
      '<h3>' + (slug ? 'Edit ' + ctx.escapeHtml(slug) : 'New upstream provider') + '</h3>' +
      '<div class="auth-form">' +
        '<label for="up-slug">Slug</label><input type="text" id="up-slug" value="' + ctx.escapeHtml(e.slug || '') + '"' + (slug ? ' readonly' : '') + ' />' +
        '<label for="up-name">Display name</label><input type="text" id="up-name" value="' + ctx.escapeHtml(e.display_name || '') + '" />' +
        '<label for="up-issuer">Issuer URL</label><input type="text" id="up-issuer" value="' + ctx.escapeHtml(e.issuer || '') + '" />' +
        '<label for="up-cid">Client ID</label><input type="text" id="up-cid" value="' + ctx.escapeHtml(e.client_id || '') + '" />' +
        '<label for="up-secret">Client secret (write-only)</label><input type="password" id="up-secret" placeholder="' + (slug ? 'leave blank to keep current' : '') + '" />' +
        '<label for="up-icon">Icon URL</label><input type="text" id="up-icon" value="' + ctx.escapeHtml(e.icon_url || '') + '" />' +
        '<label for="up-enabled">Enabled</label><input type="checkbox" id="up-enabled"' + ((e.enabled !== false) ? ' checked' : '') + ' />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="up-save">Save</button>' +
          '<button type="button" class="btn" id="up-cancel">Cancel</button>' +
        '</div>' +
      '</div>';
    document.getElementById('up-cancel').addEventListener('click', load);
    document.getElementById('up-back').addEventListener('click', load);
    document.getElementById('up-save').addEventListener('click', async function () {
      const secret = document.getElementById('up-secret').value || (e.client_secret || '');
      const body = {
        slug: document.getElementById('up-slug').value,
        display_name: document.getElementById('up-name').value,
        issuer: document.getElementById('up-issuer').value,
        client_id: document.getElementById('up-cid').value,
        client_secret: secret,
        icon_url: document.getElementById('up-icon').value || null,
        enabled: document.getElementById('up-enabled').checked,
      };
      try {
        await ctx.api.upsertOidcUpstream(body);
        ctx.toast('Saved', 'info');
        load();
      } catch (err) { ctx.toast('Save failed: ' + err.message, 'error'); }
    });
  }

  async function doDelete(slug) {
    if (!confirm('Delete upstream provider ' + slug + '?')) return;
    try {
      await ctx.api.deleteOidcUpstream(slug);
      ctx.toast('Deleted', 'info');
      load();
    } catch (err) { ctx.toast('Delete failed: ' + err.message, 'error'); }
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthOidcUpstream = { render: render };
  }

  return { render: render };
})();
