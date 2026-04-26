/* Assay Vault Console — single-page SPA covering the Phase-2..7
 * operator surfaces: Sealing, KV v2, Transit, Dynamic leases,
 * Share manager. Reuses the same admin-token storage key as the
 * Engine + Auth consoles so an operator only types it once.
 *
 * Per-pane render is a `{render(el, ctx)}` shape — same pattern the
 * Engine console uses. The shared `ctx` carries the API client +
 * UI helpers.
 */
(function () {
  'use strict';

  const ADMIN_TOKEN_KEY = 'assay-admin-token';
  const VAULT_BASE = '/api/v1/vault';
  let adminToken = localStorage.getItem(ADMIN_TOKEN_KEY) || '';
  let currentView = 'sealing';

  const tokenInput = document.getElementById('admin-token');
  const saveBtn = document.getElementById('admin-token-save');
  if (tokenInput) tokenInput.value = adminToken;
  if (saveBtn) {
    saveBtn.addEventListener('click', () => {
      adminToken = tokenInput.value.trim();
      localStorage.setItem(ADMIN_TOKEN_KEY, adminToken);
      paint();
    });
  }

  function escapeHtml(s) {
    if (s === null || s === undefined) return '';
    const d = document.createElement('div');
    d.textContent = String(s);
    return d.innerHTML;
  }

  function fmtTime(ts) {
    if (!ts) return '—';
    return new Date(ts * 1000).toLocaleString();
  }

  async function api(method, path, body) {
    const headers = { 'Content-Type': 'application/json' };
    if (adminToken) headers['Authorization'] = 'Bearer ' + adminToken;
    const init = { method, headers };
    if (body !== undefined) init.body = JSON.stringify(body);
    const r = await fetch(VAULT_BASE + path, init);
    if (r.status === 204) return null;
    const text = await r.text();
    let parsed = null;
    try { parsed = text ? JSON.parse(text) : null; } catch (_) {}
    if (!r.ok) {
      const err = new Error('HTTP ' + r.status + ' ' + path);
      err.status = r.status;
      err.body = parsed || text;
      throw err;
    }
    return parsed;
  }

  function errorBanner(msg) {
    return '<div class="error-banner">' + escapeHtml(msg) + '</div>';
  }

  function statusCard(title, value, pillClass) {
    const v = pillClass
      ? '<span class="pill ' + pillClass + '">' + escapeHtml(value) + '</span>'
      : '<div class="value">' + escapeHtml(value) + '</div>';
    return '<div class="card"><h3>' + escapeHtml(title) + '</h3>' + v + '</div>';
  }

  // ───────────── Panes ─────────────
  const Sealing = {
    async render(el) {
      el.innerHTML = '<h1>Sealing</h1><p class="placeholder">Loading…</p>';
      try {
        const status = await api('GET', '/sys/seal-status');
        const sealedPill = status.sealed ? 'pill-sealed' : 'pill-unsealed';
        const sealedTxt = status.sealed ? 'sealed' : 'unsealed';
        const cards =
          statusCard('State', sealedTxt, sealedPill) +
          statusCard('Method', status.method || '—') +
          statusCard('KEK kid', status.kid || '—') +
          (status.share_threshold
            ? statusCard('Threshold', status.shares_progress + ' / ' + status.share_threshold)
            : '');

        const actionsHtml =
          '<div class="toolbar">' +
          (status.sealed
            ? '<input type="text" id="unseal-share" placeholder="share_b64">' +
              '<button class="btn btn-primary" id="btn-unseal">Submit share</button>'
            : '<button class="btn btn-danger" id="btn-seal">Seal vault</button>') +
          '<button class="btn" id="btn-init">Init shamir…</button>' +
          '</div>';

        el.innerHTML =
          '<h1>Sealing</h1>' +
          '<div class="pane-status">' + cards + '</div>' +
          actionsHtml;

        const sealBtn = document.getElementById('btn-seal');
        if (sealBtn) {
          sealBtn.addEventListener('click', async () => {
            try {
              await api('POST', '/sys/seal');
              await Sealing.render(el);
            } catch (e) {
              el.insertAdjacentHTML('afterbegin', errorBanner('seal: ' + e.message));
            }
          });
        }
        const unsealBtn = document.getElementById('btn-unseal');
        if (unsealBtn) {
          unsealBtn.addEventListener('click', async () => {
            const share = (document.getElementById('unseal-share').value || '').trim();
            if (!share) return;
            try {
              await api('POST', '/sys/unseal', { share_b64: share });
              await Sealing.render(el);
            } catch (e) {
              el.insertAdjacentHTML('afterbegin', errorBanner('unseal: ' + e.message));
            }
          });
        }
        const initBtn = document.getElementById('btn-init');
        if (initBtn) {
          initBtn.addEventListener('click', async () => {
            const t = parseInt(prompt('Threshold (e.g. 3):'), 10);
            const n = parseInt(prompt('Shares (e.g. 5):'), 10);
            if (!t || !n) return;
            try {
              const out = await api('POST', '/sys/init', { threshold: t, shares_count: n });
              alert(
                'Shamir init complete. Save these shares — the engine WILL NOT show them again.\n\n' +
                  out.shares_b64.join('\n')
              );
              await Sealing.render(el);
            } catch (e) {
              el.insertAdjacentHTML('afterbegin', errorBanner('init: ' + e.message));
            }
          });
        }
      } catch (e) {
        el.innerHTML = '<h1>Sealing</h1>' + errorBanner('seal-status: ' + e.message);
      }
    },
  };

  const Kv = {
    async render(el) {
      el.innerHTML =
        '<h1>KV v2</h1>' +
        '<div class="toolbar">' +
        '<input type="text" id="kv-prefix" placeholder="prefix (empty = all)">' +
        '<button class="btn btn-primary" id="kv-list">List</button>' +
        '<input type="text" id="kv-path" placeholder="path">' +
        '<input type="text" id="kv-value" placeholder="value">' +
        '<button class="btn" id="kv-put">PUT</button>' +
        '</div>' +
        '<div id="kv-result"></div>';

      async function listKv() {
        const prefix = document.getElementById('kv-prefix').value.trim();
        const path = prefix === '' ? '/kv-list' : '/kv-list/' + encodeURIComponent(prefix);
        try {
          const out = await api('GET', path);
          renderEntries(out.entries || []);
        } catch (e) {
          document.getElementById('kv-result').innerHTML = errorBanner(e.message);
        }
      }

      function renderEntries(entries) {
        let html =
          '<table class="data"><thead><tr><th>Path</th><th>Latest</th>' +
          '<th>Updated</th><th></th></tr></thead><tbody>';
        for (const m of entries) {
          html +=
            '<tr><td>' + escapeHtml(m.path) + '</td>' +
            '<td>' + m.latest_version + '</td>' +
            '<td>' + fmtTime(m.updated_at) + '</td>' +
            '<td><button class="btn" data-get="' + escapeHtml(m.path) + '">GET</button></td></tr>';
        }
        html += '</tbody></table>';
        const out = document.getElementById('kv-result');
        out.innerHTML = html;
        out.querySelectorAll('button[data-get]').forEach((b) => {
          b.addEventListener('click', async () => {
            const p = b.getAttribute('data-get');
            try {
              const r = await api('GET', '/kv/' + p);
              alert(JSON.stringify(r, null, 2));
            } catch (e) {
              alert('GET failed: ' + e.message);
            }
          });
        });
      }

      document.getElementById('kv-list').addEventListener('click', listKv);
      document.getElementById('kv-put').addEventListener('click', async () => {
        const path = document.getElementById('kv-path').value.trim();
        const data = document.getElementById('kv-value').value;
        if (!path) return;
        try {
          await api('PUT', '/kv/' + path, { data, custom_md: {} });
          await listKv();
        } catch (e) {
          document.getElementById('kv-result').innerHTML = errorBanner('PUT: ' + e.message);
        }
      });
      await listKv();
    },
  };

  const Transit = {
    async render(el) {
      el.innerHTML =
        '<h1>Transit</h1>' +
        '<div class="toolbar">' +
        '<input type="text" id="tr-name" placeholder="name">' +
        '<button class="btn btn-primary" id="tr-create">Create</button>' +
        '<button class="btn" id="tr-rotate">Rotate</button>' +
        '</div>' +
        '<div id="tr-result"></div>';

      async function refresh() {
        try {
          const out = await api('GET', '/transit/keys');
          let html =
            '<table class="data"><thead><tr><th>Name</th><th>Latest</th><th>Algo</th>' +
            '<th>Created</th></tr></thead><tbody>';
          for (const k of (out.keys || [])) {
            html +=
              '<tr><td>' + escapeHtml(k.name) + '</td>' +
              '<td>' + k.latest_ver + '</td>' +
              '<td>' + escapeHtml(k.algo) + '</td>' +
              '<td>' + fmtTime(k.created_at) + '</td></tr>';
          }
          html += '</tbody></table>';
          document.getElementById('tr-result').innerHTML = html;
        } catch (e) {
          document.getElementById('tr-result').innerHTML = errorBanner(e.message);
        }
      }

      document.getElementById('tr-create').addEventListener('click', async () => {
        const name = document.getElementById('tr-name').value.trim();
        if (!name) return;
        try {
          await api('POST', '/transit/keys/' + encodeURIComponent(name), {});
          await refresh();
        } catch (e) {
          document.getElementById('tr-result').innerHTML = errorBanner('create: ' + e.message);
        }
      });
      document.getElementById('tr-rotate').addEventListener('click', async () => {
        const name = document.getElementById('tr-name').value.trim();
        if (!name) return;
        try {
          await api('POST', '/transit/keys/' + encodeURIComponent(name) + '/rotate', {});
          await refresh();
        } catch (e) {
          document.getElementById('tr-result').innerHTML = errorBanner('rotate: ' + e.message);
        }
      });
      await refresh();
    },
  };

  const Leases = {
    async render(el) {
      el.innerHTML =
        '<h1>Dynamic leases</h1>' +
        '<div class="toolbar">' +
        '<input type="text" id="ls-provider" placeholder="provider filter (postgres/aws/gcp/kubernetes)">' +
        '<button class="btn btn-primary" id="ls-list">List</button>' +
        '</div>' +
        '<div id="ls-result"></div>';

      async function refresh() {
        const prov = document.getElementById('ls-provider').value.trim();
        const url = prov ? '/dynamic/leases?provider=' + encodeURIComponent(prov) : '/dynamic/leases';
        try {
          const out = await api('GET', url);
          let html =
            '<table class="data"><thead><tr><th>ID</th><th>Provider</th><th>Role</th>' +
            '<th>Issued</th><th>Expires</th><th>Revoked</th><th></th></tr></thead><tbody>';
          for (const l of (out.leases || [])) {
            html +=
              '<tr><td>' + escapeHtml(l.id) + '</td>' +
              '<td>' + escapeHtml(l.provider) + '</td>' +
              '<td>' + escapeHtml(l.role) + '</td>' +
              '<td>' + fmtTime(l.issued_at) + '</td>' +
              '<td>' + fmtTime(l.expires_at) + '</td>' +
              '<td>' + (l.revoked_at ? fmtTime(l.revoked_at) : '—') + '</td>' +
              '<td><button class="btn btn-danger" data-revoke="' + escapeHtml(l.id) + '">Revoke</button></td></tr>';
          }
          html += '</tbody></table>';
          document.getElementById('ls-result').innerHTML = html;
          document.querySelectorAll('button[data-revoke]').forEach((b) => {
            b.addEventListener('click', async () => {
              const id = b.getAttribute('data-revoke');
              try {
                await api('DELETE', '/dynamic/leases/' + id);
                await refresh();
              } catch (e) {
                alert('revoke: ' + e.message);
              }
            });
          });
        } catch (e) {
          document.getElementById('ls-result').innerHTML = errorBanner(e.message);
        }
      }

      document.getElementById('ls-list').addEventListener('click', refresh);
      await refresh();
    },
  };

  const Share = {
    async render(el) {
      el.innerHTML =
        '<h1>Share manager</h1>' +
        '<div class="toolbar">' +
        '<select id="sh-kind"><option>item</option><option>vault</option><option>collection</option></select>' +
        '<input type="text" id="sh-id" placeholder="target id">' +
        '<input type="text" id="sh-ttl" placeholder="ttl seconds (e.g. 3600)">' +
        '<button class="btn btn-primary" id="sh-mint">Mint</button>' +
        '</div>' +
        '<div id="sh-mint-result"></div>' +
        '<div class="toolbar">' +
        '<input type="text" id="sh-rev" placeholder="revocation_id (hex)">' +
        '<input type="text" id="sh-reason" placeholder="reason">' +
        '<button class="btn btn-danger" id="sh-revoke">Revoke</button>' +
        '</div>' +
        '<div id="sh-revoke-result"></div>';

      document.getElementById('sh-mint').addEventListener('click', async () => {
        const kind = document.getElementById('sh-kind').value;
        const id = document.getElementById('sh-id').value.trim();
        const ttl = parseInt(document.getElementById('sh-ttl').value, 10) || 3600;
        if (!id) return;
        try {
          const out = await api('POST', '/share', {
            target_kind: kind,
            target_id: id,
            ttl_secs: ttl,
          });
          document.getElementById('sh-mint-result').innerHTML =
            '<pre class="json">' + escapeHtml(JSON.stringify(out, null, 2)) + '</pre>';
        } catch (e) {
          document.getElementById('sh-mint-result').innerHTML = errorBanner('mint: ' + e.message);
        }
      });
      document.getElementById('sh-revoke').addEventListener('click', async () => {
        const rid = document.getElementById('sh-rev').value.trim();
        const reason = document.getElementById('sh-reason').value.trim();
        if (!rid) return;
        try {
          await api('POST', '/share/revoke', { revocation_id: rid, reason });
          document.getElementById('sh-revoke-result').innerHTML =
            '<div class="error-banner" style="background:#1a3;border-color:#181;color:#cfc;">' +
            'Revoked: ' + escapeHtml(rid) + '</div>';
        } catch (e) {
          document.getElementById('sh-revoke-result').innerHTML = errorBanner('revoke: ' + e.message);
        }
      });
    },
  };

  const PANES = {
    sealing: Sealing,
    kv: Kv,
    transit: Transit,
    leases: Leases,
    share: Share,
  };

  function paint() {
    const el = document.getElementById('content');
    const pane = PANES[currentView] || Sealing;
    document.querySelectorAll('.sidebar-nav a').forEach((a) => {
      a.classList.toggle('active', a.getAttribute('data-view') === currentView);
    });
    pane.render(el);
  }

  document.querySelectorAll('.sidebar-nav a').forEach((a) => {
    a.addEventListener('click', (e) => {
      e.preventDefault();
      currentView = a.getAttribute('data-view');
      window.location.hash = currentView;
      paint();
    });
  });

  // Bootstrap from URL hash if present.
  const hash = window.location.hash.replace('#', '');
  if (hash && PANES[hash]) currentView = hash;

  paint();
})();
