/* Zanzibar pane — namespace browser, tuple write/delete, check evaluator, expand viewer. */

var AssayAuthZanzibar = (function () {
  'use strict';

  let ctx = null;
  let container = null;
  let activeTab = 'namespaces';

  function render(el, c) {
    ctx = c;
    container = el;
    container.innerHTML =
      '<div class="auth-toolbar">' +
        '<h2 class="section-title">Zanzibar / ReBAC</h2>' +
      '</div>' +
      '<div class="auth-toolbar" style="border-bottom:1px solid var(--border); padding-bottom:8px;">' +
        '<button type="button" class="btn btn-small" data-tab="namespaces">Namespaces</button>' +
        '<button type="button" class="btn btn-small" data-tab="check">Check</button>' +
        '<button type="button" class="btn btn-small" data-tab="expand">Expand</button>' +
        '<button type="button" class="btn btn-small" data-tab="tuple">Write tuple</button>' +
      '</div>' +
      '<div id="zb-wrap"></div>';

    container.addEventListener('click', function (e) {
      const tab = e.target.closest('[data-tab]');
      if (tab) {
        activeTab = tab.dataset.tab;
        renderTab();
        return;
      }
      const action = e.target.closest('[data-action]');
      if (action) {
        const a = action.dataset.action;
        if (a === 'view-ns') return showNamespace(action.dataset.name);
      }
    });

    renderTab();
  }

  function renderTab() {
    if (activeTab === 'namespaces') renderNamespaces();
    else if (activeTab === 'check') renderCheckForm();
    else if (activeTab === 'expand') renderExpandForm();
    else if (activeTab === 'tuple') renderTupleForm();
  }

  async function renderNamespaces() {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<div class="auth-empty">Loading namespaces…</div>';
    try {
      const items = await ctx.api.listZanzibarNamespaces();
      if (!items || !items.length) {
        wrap.innerHTML = '<div class="auth-empty">No namespaces defined.</div>';
        return;
      }
      let html = '<table class="data-table"><thead><tr><th>Name</th><th></th></tr></thead><tbody>';
      items.forEach(function (ns) {
        html += '<tr><td class="auth-mono">' + ctx.escapeHtml(ns.name) +
          '</td><td><button class="btn btn-small" data-action="view-ns" data-name="' + ctx.escapeHtml(ns.name) + '">View</button></td></tr>';
      });
      html += '</tbody></table>';
      wrap.innerHTML = html;
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  async function showNamespace(name) {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<div class="auth-empty">Loading…</div>';
    try {
      const ns = await ctx.api.getZanzibarNamespace(name);
      wrap.innerHTML = '<button class="btn btn-small" id="zb-back">&larr; Back</button>' +
        '<h3>' + ctx.escapeHtml(ns.name) + '</h3>' +
        '<pre class="auth-tree">' + ctx.escapeHtml(JSON.stringify(ns, null, 2)) + '</pre>';
      document.getElementById('zb-back').addEventListener('click', renderNamespaces);
    } catch (err) {
      wrap.innerHTML = '<div class="auth-empty">Error: ' + ctx.escapeHtml(err.message) + '</div>';
    }
  }

  function renderCheckForm() {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<h3>Permission check</h3>' +
      '<p class="auth-empty">Does <code>subject</code> have <code>permission</code> on <code>resource</code>?</p>' +
      '<div class="auth-form">' +
        '<label for="zc-rt">Resource type</label><input type="text" id="zc-rt" placeholder="document" />' +
        '<label for="zc-rid">Resource id</label><input type="text" id="zc-rid" placeholder="readme" />' +
        '<label for="zc-perm">Permission</label><input type="text" id="zc-perm" placeholder="view" />' +
        '<label for="zc-st">Subject type</label><input type="text" id="zc-st" placeholder="user" />' +
        '<label for="zc-sid">Subject id</label><input type="text" id="zc-sid" placeholder="alice" />' +
        '<label for="zc-srel">Subject relation (optional)</label><input type="text" id="zc-srel" placeholder="member" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="zc-run">Check</button>' +
        '</div>' +
      '</div>' +
      '<div id="zc-result" style="margin-top:16px;"></div>';
    document.getElementById('zc-run').addEventListener('click', async function () {
      const body = {
        resource_type: document.getElementById('zc-rt').value,
        resource_id: document.getElementById('zc-rid').value,
        permission: document.getElementById('zc-perm').value,
        subject_type: document.getElementById('zc-st').value,
        subject_id: document.getElementById('zc-sid').value,
        subject_rel: document.getElementById('zc-srel').value || null,
      };
      try {
        const r = await ctx.api.checkZanzibar(body);
        const cls = r.allowed ? 'auth-status-allowed' : (r.result === 'Denied' ? 'auth-status-denied' : 'auth-status-other');
        document.getElementById('zc-result').innerHTML =
          '<p>Result: <span class="' + cls + '">' + ctx.escapeHtml(r.result) + '</span></p>';
      } catch (err) {
        document.getElementById('zc-result').innerHTML = '<p class="auth-status-denied">Error: ' + ctx.escapeHtml(err.message) + '</p>';
      }
    });
  }

  function renderExpandForm() {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<h3>Userset expand</h3>' +
      '<p class="auth-empty">Show every subject that satisfies <code>relation</code> on <code>resource</code>.</p>' +
      '<div class="auth-form">' +
        '<label for="ze-rt">Resource type</label><input type="text" id="ze-rt" />' +
        '<label for="ze-rid">Resource id</label><input type="text" id="ze-rid" />' +
        '<label for="ze-rel">Relation</label><input type="text" id="ze-rel" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="ze-run">Expand</button>' +
        '</div>' +
      '</div>' +
      '<div id="ze-result" style="margin-top:16px;"></div>';
    document.getElementById('ze-run').addEventListener('click', async function () {
      const body = {
        resource_type: document.getElementById('ze-rt').value,
        resource_id: document.getElementById('ze-rid').value,
        relation: document.getElementById('ze-rel').value,
      };
      try {
        const tree = await ctx.api.expandZanzibar(body);
        document.getElementById('ze-result').innerHTML =
          '<pre class="auth-tree">' + ctx.escapeHtml(JSON.stringify(tree, null, 2)) + '</pre>';
      } catch (err) {
        document.getElementById('ze-result').innerHTML = '<p class="auth-status-denied">Error: ' + ctx.escapeHtml(err.message) + '</p>';
      }
    });
  }

  function renderTupleForm() {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<h3>Write / delete tuple</h3>' +
      '<div class="auth-form">' +
        '<label for="zt-ot">Object type</label><input type="text" id="zt-ot" />' +
        '<label for="zt-oid">Object id</label><input type="text" id="zt-oid" />' +
        '<label for="zt-rel">Relation</label><input type="text" id="zt-rel" />' +
        '<label for="zt-st">Subject type</label><input type="text" id="zt-st" />' +
        '<label for="zt-sid">Subject id</label><input type="text" id="zt-sid" />' +
        '<label for="zt-srel">Subject relation (optional)</label><input type="text" id="zt-srel" />' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="zt-write">Write</button>' +
          '<button type="button" class="btn btn-danger" id="zt-delete">Delete</button>' +
        '</div>' +
      '</div>' +
      '<div id="zt-result" style="margin-top:16px;"></div>';
    function body() {
      return {
        object_type: document.getElementById('zt-ot').value,
        object_id: document.getElementById('zt-oid').value,
        relation: document.getElementById('zt-rel').value,
        subject_type: document.getElementById('zt-st').value,
        subject_id: document.getElementById('zt-sid').value,
        subject_rel: document.getElementById('zt-srel').value || null,
      };
    }
    document.getElementById('zt-write').addEventListener('click', async function () {
      try {
        await ctx.api.writeZanzibarTuple(body());
        document.getElementById('zt-result').innerHTML = '<p class="auth-status-allowed">Tuple written.</p>';
      } catch (err) {
        document.getElementById('zt-result').innerHTML = '<p class="auth-status-denied">Error: ' + ctx.escapeHtml(err.message) + '</p>';
      }
    });
    document.getElementById('zt-delete').addEventListener('click', async function () {
      try {
        await ctx.api.deleteZanzibarTuple(body());
        document.getElementById('zt-result').innerHTML = '<p class="auth-status-allowed">Tuple deleted.</p>';
      } catch (err) {
        document.getElementById('zt-result').innerHTML = '<p class="auth-status-denied">Error: ' + ctx.escapeHtml(err.message) + '</p>';
      }
    });
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthZanzibar = { render: render };
  }

  return { render: render };
})();
