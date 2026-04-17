/* Assay Workflow Dashboard - Settings Component */

var AssaySettings = (function () {
  'use strict';

  let ctx = null;
  let container = null;

  function render(el, context) {
    ctx = context;
    container = el;

    el.innerHTML =
      '<h2 class="section-title">Settings</h2>' +
      '<div class="card" style="margin-bottom: 24px;">' +
        '<div class="card-header">' +
          '<span>Namespaces</span>' +
          '<button class="btn btn-sm btn-primary" id="settings-toggle-create">+ Create</button>' +
        '</div>' +
        '<div class="card-body">' +
          '<div id="settings-create-wrap" style="margin-bottom: 12px;"></div>' +
          '<div id="settings-ns-table"></div>' +
        '</div>' +
      '</div>' +
      '<div class="card">' +
        '<div class="card-header">Engine Info</div>' +
        '<div class="card-body" id="settings-engine-info">' +
          '<div class="meta-grid">' +
            metaItem('Service', 'assay-workflow') +
            metaItem('Version', '<span class="mono" id="settings-version">loading…</span>') +
            metaItem('Build', '<span id="settings-build-profile">—</span>') +
            metaItem('API Docs', '<a href="/api/v1/docs" target="_blank" class="clickable">/api/v1/docs</a>') +
            metaItem('OpenAPI Spec', '<a href="/api/v1/openapi.json" target="_blank" class="clickable">/api/v1/openapi.json</a>') +
          '</div>' +
        '</div>' +
      '</div>';

    loadEngineInfo();

    var showCreate = false;
    el.querySelector('#settings-toggle-create').addEventListener('click', function () {
      showCreate = !showCreate;
      renderCreateForm(showCreate);
    });

    el.querySelector('#settings-ns-table').addEventListener('click', function (e) {
      var btn = e.target.closest('.btn-delete-ns');
      if (btn) {
        e.preventDefault();
        handleDeleteNs(btn.dataset.name);
      }
    });

    loadNamespaces();
  }

  function renderCreateForm(show) {
    var wrap = container.querySelector('#settings-create-wrap');
    if (!show) {
      wrap.innerHTML = '';
      return;
    }
    wrap.innerHTML =
      '<div class="form-inline">' +
        '<div class="form-group" style="flex: 1; margin-bottom: 0;">' +
          '<label class="form-label">Namespace Name</label>' +
          '<input type="text" class="form-input" id="settings-ns-name" placeholder="production">' +
        '</div>' +
        '<button class="btn btn-primary" id="settings-create-ns-btn">Create</button>' +
      '</div>';

    wrap.querySelector('#settings-create-ns-btn').addEventListener('click', handleCreateNs);
    wrap.querySelector('#settings-ns-name').addEventListener('keydown', function (e) {
      if (e.key === 'Enter') handleCreateNs();
    });
  }

  async function loadNamespaces() {
    var tableWrap = container.querySelector('#settings-ns-table');
    try {
      var namespaces = await fetch('/api/v1/namespaces').then(function (r) { return r.json(); });
      var statsPromises = namespaces.map(async function (ns) {
        var name = ns.name || ns;
        try {
          var r = await fetch('/api/v1/namespaces/' + encodeURIComponent(name));
          return await r.json();
        } catch (_) {
          return { namespace: name, total_workflows: 0, schedules: 0, workers: 0 };
        }
      });

      var allStats = await Promise.all(statsPromises);
      renderNsTable(tableWrap, namespaces, allStats);
    } catch (err) {
      tableWrap.innerHTML = '<div class="empty-state"><p>Error: ' + ctx.escapeHtml(err.message) + '</p></div>';
    }
  }

  function renderNsTable(wrap, namespaces, allStats) {
    if (namespaces.length === 0) {
      wrap.innerHTML = '<div class="empty-state"><p>No namespaces</p></div>';
      return;
    }

    var html =
      '<table class="data-table"><thead><tr>' +
        '<th>Name</th>' +
        '<th>Workflows</th>' +
        '<th>Schedules</th>' +
        '<th>Workers</th>' +
        '<th>Created</th>' +
        '<th>Actions</th>' +
      '</tr></thead><tbody>';

    for (var i = 0; i < namespaces.length; i++) {
      var ns = namespaces[i];
      var name = ns.name || ns;
      var stats = allStats[i] || {};
      var isMain = name === 'main';

      html +=
        '<tr>' +
          '<td class="mono">' + ctx.escapeHtml(name) + '</td>' +
          '<td>' + (stats.total_workflows || 0) + '</td>' +
          '<td>' + (stats.schedules || 0) + '</td>' +
          '<td>' + (stats.workers || 0) + '</td>' +
          '<td>' + (ns.created_at ? ctx.formatTime(ns.created_at) : '-') + '</td>' +
          '<td>' +
            (isMain
              ? '<button class="btn btn-sm btn-danger" disabled title="Cannot delete main namespace">Delete</button>'
              : '<button class="btn btn-sm btn-danger btn-delete-ns" data-name="' + ctx.escapeHtml(name) + '">Delete</button>') +
          '</td>' +
        '</tr>';
    }

    html += '</tbody></table>';
    wrap.innerHTML = html;
  }

  async function handleCreateNs() {
    var input = container.querySelector('#settings-ns-name');
    var name = input.value.trim();
    if (!name) {
      ctx.toast('Namespace name is required', 'error');
      return;
    }

    try {
      var res = await fetch('/api/v1/namespaces', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: name }),
      });
      if (!res.ok) {
        var body = await res.text();
        throw new Error(body || res.statusText);
      }
      input.value = '';
      ctx.toast("Created namespace '" + name + "'", 'success');
      loadNamespaces();
      // Refresh the namespace dropdown in the sidebar.
      if (window.AssayApp && window.AssayApp.refreshCurrentView) {
        window.AssayApp.refreshCurrentView();
      }
    } catch (err) {
      ctx.toast('Create failed: ' + err.message, 'error');
    }
  }

  async function handleDeleteNs(name) {
    if (!confirm('Delete namespace "' + name + '"? This cannot be undone.')) return;
    try {
      var res = await fetch('/api/v1/namespaces/' + encodeURIComponent(name), { method: 'DELETE' });
      if (!res.ok) {
        var body = await res.text();
        throw new Error(body || res.statusText);
      }
      ctx.toast("Deleted namespace '" + name + "'", 'success');
      loadNamespaces();
      if (window.AssayApp && window.AssayApp.refreshCurrentView) {
        window.AssayApp.refreshCurrentView();
      }
    } catch (err) {
      ctx.toast('Delete failed: ' + err.message, 'error');
    }
  }

  async function loadEngineInfo() {
    try {
      var v = await ctx.apiFetchRaw('/version');
      if (v) {
        var vel = container.querySelector('#settings-version');
        var bel = container.querySelector('#settings-build-profile');
        if (vel) vel.textContent = 'v' + (v.version || '?');
        if (bel) bel.textContent = v.build_profile || '—';
      }
    } catch (_) {
      // Non-fatal — older engines may not have /version. Leave placeholders.
    }
  }

  function metaItem(label, value) {
    return '<div class="meta-item"><label>' + label + '</label><span>' + value + '</span></div>';
  }

  return { render: render };
})();
