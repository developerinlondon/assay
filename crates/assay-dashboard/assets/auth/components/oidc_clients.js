/* OIDC Clients pane — CRUD over /admin/oidc/clients with show-secret-once. */

var AssayAuthOidcClients = (function () {
  'use strict';

  let ctx = null;
  let container = null;

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">OIDC Clients</h2>' +
        '<button type="button" class="btn btn-primary" id="oc-new">New client</button>' +
      '</div>' +
      '<div id="oc-wrap"><div class="auth-empty">Loading…</div></div>';

    document.getElementById('oc-new').addEventListener('click', openCreate);

    container.addEventListener('click', function (e) {
      const action = e.target.closest('[data-action]');
      if (!action) return;
      const a = action.dataset.action;
      const id = action.dataset.id;
      if (a === 'view') return showDetail(id);
      if (a === 'rotate') return rotate(id);
      if (a === 'delete') return doDelete(id);
    });

    load();
  }

  async function load() {
    const wrap = container.querySelector('#oc-wrap');
    try {
      const items = await ctx.api.listOidcClients();
      renderTable(wrap, items || []);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderTable(wrap, items) {
    if (!items.length) {
      wrap.innerHTML = '<div class="auth-empty">No clients registered.</div>';
      return;
    }
    let html = '<table class="data-table"><thead><tr>' +
      '<th>Client ID</th><th>Name</th><th>Auth method</th><th>Has secret</th><th>Redirect URIs</th><th></th>' +
      '</tr></thead><tbody>';
    items.forEach(function (c) {
      html += '<tr>' +
        '<td class="auth-mono">' + ctx.escapeHtml(ctx.truncate(c.client_id, 22)) + '</td>' +
        '<td>' + ctx.escapeHtml(c.name) + '</td>' +
        '<td>' + ctx.escapeHtml(c.token_endpoint_auth_method) + '</td>' +
        '<td>' + (c.client_secret_hash ? '✓' : '—') + '</td>' +
        '<td class="auth-mono">' + ctx.escapeHtml((c.redirect_uris || []).join(', ')) + '</td>' +
        '<td>' +
          '<button class="btn btn-small" data-action="view" data-id="' + ctx.escapeHtml(c.client_id) + '">View</button> ' +
          '<button class="btn btn-small" data-action="rotate" data-id="' + ctx.escapeHtml(c.client_id) + '">Rotate</button> ' +
          '<button class="btn btn-small btn-danger" data-action="delete" data-id="' + ctx.escapeHtml(c.client_id) + '">Delete</button>' +
        '</td>' +
      '</tr>';
    });
    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  function openCreate() {
    const wrap = container.querySelector('#oc-wrap');
    wrap.innerHTML = '<h3>New OIDC client</h3>' +
      '<div class="auth-form">' +
        '<label for="nc-name">Name</label><input type="text" id="nc-name" />' +
        '<label for="nc-redirect">Redirect URIs (one per line)</label><textarea id="nc-redirect" placeholder="https://app.example/callback"></textarea>' +
        '<label for="nc-method">Auth method</label><select id="nc-method">' +
          '<option value="client_secret_basic" selected>client_secret_basic</option>' +
          '<option value="client_secret_post">client_secret_post</option>' +
          '<option value="none">none (PKCE-only)</option>' +
        '</select>' +
        '<label for="nc-scopes">Default scopes (space-sep)</label><input type="text" id="nc-scopes" value="openid email profile" />' +
        '<label for="nc-grants">Grant types (comma-sep)</label><input type="text" id="nc-grants" value="authorization_code,refresh_token" />' +
        '<label for="nc-resp">Response types (comma-sep)</label><input type="text" id="nc-resp" value="code" />' +
        '<label for="nc-pkce">PKCE required</label><input type="checkbox" id="nc-pkce" checked />' +
        '<label for="nc-consent">Require consent</label><input type="checkbox" id="nc-consent" checked />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="nc-create">Create</button>' +
          '<button type="button" class="btn" id="nc-cancel">Cancel</button>' +
        '</div>' +
      '</div>';
    document.getElementById('nc-cancel').addEventListener('click', load);
    document.getElementById('nc-create').addEventListener('click', async function () {
      const body = {
        name: document.getElementById('nc-name').value,
        redirect_uris: document.getElementById('nc-redirect').value.split('\n').map(function (s) { return s.trim(); }).filter(Boolean),
        token_endpoint_auth_method: document.getElementById('nc-method').value,
        default_scopes: document.getElementById('nc-scopes').value.split(/\s+/).filter(Boolean),
        grant_types: document.getElementById('nc-grants').value.split(',').map(function (s) { return s.trim(); }).filter(Boolean),
        response_types: document.getElementById('nc-resp').value.split(',').map(function (s) { return s.trim(); }).filter(Boolean),
        pkce_required: document.getElementById('nc-pkce').checked,
        require_consent: document.getElementById('nc-consent').checked,
      };
      try {
        const result = await ctx.api.createOidcClient(body);
        renderSecretOnce(result, true);
      } catch (err) {
        ctx.toast('Create failed: ' + err.message, 'error');
      }
    });
  }

  function renderSecretOnce(result, isCreate) {
    const wrap = container.querySelector('#oc-wrap');
    const secret = isCreate ? result.client_secret : result.client_secret;
    let html = '<button class="btn btn-small" id="oc-back">&larr; Back</button>' +
      '<h3>Client ' + (isCreate ? 'created' : 'secret rotated') + '</h3>';
    if (secret) {
      html += '<div class="auth-secret-once">' +
        '<strong>Capture this client_secret now — it will not be shown again.</strong>' +
        '<span class="auth-mono">' + ctx.escapeHtml(secret) + '</span>' +
        '<button type="button" class="btn btn-small" id="oc-copy">Copy</button>' +
      '</div>';
    }
    if (isCreate) {
      html += '<p>Client ID: <span class="auth-mono">' + ctx.escapeHtml(result.client.client_id) + '</span></p>';
    } else {
      html += '<p>Client ID: <span class="auth-mono">' + ctx.escapeHtml(result.client_id) + '</span></p>';
    }
    wrap.innerHTML = html;
    document.getElementById('oc-back').addEventListener('click', load);
    const copy = document.getElementById('oc-copy');
    if (copy) copy.addEventListener('click', function () {
      navigator.clipboard.writeText(secret).then(function () { ctx.toast('Copied', 'info'); });
    });
  }

  async function showDetail(id) {
    const wrap = container.querySelector('#oc-wrap');
    wrap.innerHTML = '<div class="auth-empty">Loading…</div>';
    try {
      const c = await ctx.api.getOidcClient(id);
      let html = '<button class="btn btn-small" id="oc-back">&larr; Back</button>' +
        '<h3>' + ctx.escapeHtml(c.name) + '</h3>' +
        '<dl class="auth-form">' +
          '<dt>Client ID</dt><dd class="auth-mono">' + ctx.escapeHtml(c.client_id) + '</dd>' +
          '<dt>Has secret</dt><dd>' + (c.client_secret_hash ? 'yes (Argon2id PHC)' : 'no (PKCE-only)') + '</dd>' +
          '<dt>Redirect URIs</dt><dd>' + (c.redirect_uris || []).map(function (u) { return '<div class="auth-mono">' + ctx.escapeHtml(u) + '</div>'; }).join('') + '</dd>' +
          '<dt>Auth method</dt><dd>' + ctx.escapeHtml(c.token_endpoint_auth_method) + '</dd>' +
          '<dt>Default scopes</dt><dd>' + ctx.escapeHtml((c.default_scopes || []).join(' ')) + '</dd>' +
          '<dt>Grant types</dt><dd>' + ctx.escapeHtml((c.grant_types || []).join(', ')) + '</dd>' +
          '<dt>Response types</dt><dd>' + ctx.escapeHtml((c.response_types || []).join(', ')) + '</dd>' +
          '<dt>PKCE required</dt><dd>' + (c.pkce_required ? '✓' : '—') + '</dd>' +
          '<dt>Require consent</dt><dd>' + (c.require_consent ? '✓' : '—') + '</dd>' +
          '<dt>Backchannel logout</dt><dd class="auth-mono">' + ctx.escapeHtml(c.backchannel_logout_uri || '—') + '</dd>' +
        '</dl>';
      wrap.innerHTML = html;
      document.getElementById('oc-back').addEventListener('click', load);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  async function rotate(id) {
    if (!confirm('Rotate client_secret for ' + id + '? Existing instances using the old secret will start failing immediately.')) return;
    try {
      const result = await ctx.api.rotateOidcClientSecret(id);
      renderSecretOnce(result, false);
    } catch (err) { ctx.toast('Rotate failed: ' + err.message, 'error'); }
  }

  async function doDelete(id) {
    if (!confirm('Delete client ' + id + '?')) return;
    try {
      await ctx.api.deleteOidcClient(id);
      ctx.toast('Deleted', 'info');
      load();
    } catch (err) { ctx.toast('Delete failed: ' + err.message, 'error'); }
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthOidcClients = { render: render };
  }

  return { render: render };
})();
