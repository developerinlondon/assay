/* Zanzibar pane — namespace browser + define, tuple write/delete, check evaluator, expand viewer, bootstrap admin. */

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
        '<button type="button" class="btn btn-small" data-tab="define">Define namespace</button>' +
        '<button type="button" class="btn btn-small" data-tab="check">Check</button>' +
        '<button type="button" class="btn btn-small" data-tab="expand">Expand</button>' +
        '<button type="button" class="btn btn-small" data-tab="tuple">Write tuple</button>' +
        '<button type="button" class="btn btn-small" data-tab="bootstrap">Bootstrap admin</button>' +
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
    else if (activeTab === 'define') renderDefineForm();
    else if (activeTab === 'check') renderCheckForm();
    else if (activeTab === 'expand') renderExpandForm();
    else if (activeTab === 'tuple') renderTupleForm();
    else if (activeTab === 'bootstrap') renderBootstrapForm();
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

  function renderDefineForm() {
    const wrap = container.querySelector('#zb-wrap');
    wrap.innerHTML = '<h3>Define / replace namespace</h3>' +
      '<p class="auth-empty">Paste a JSON schema. POSTed to <code>/admin/zanzibar/namespaces</code>; idempotent (replaces existing). See the Namespaces tab for the current shape of any defined namespace.</p>' +
      '<div class="auth-form">' +
        '<label for="zd-json">Schema (JSON)</label>' +
        '<textarea id="zd-json" rows="14" spellcheck="false" ' +
          'style="font-family:var(--mono); font-size:13px; width:100%;" ' +
          'placeholder=\'{\n  "name": "document",\n  "relations": { "viewer": {}, "editor": {} }\n}\'></textarea>' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-small" id="zd-validate">Validate JSON</button>' +
          '<button type="button" class="btn btn-primary" id="zd-submit">Define</button>' +
        '</div>' +
      '</div>' +
      '<div id="zd-result" style="margin-top:16px;"></div>';

    function setResult(cls, msg) {
      document.getElementById('zd-result').innerHTML =
        '<p class="' + cls + '">' + ctx.escapeHtml(msg) + '</p>';
    }
    function parse() {
      const raw = document.getElementById('zd-json').value.trim();
      if (!raw) throw new Error('schema is empty');
      try { return JSON.parse(raw); }
      catch (e) { throw new Error('invalid JSON: ' + e.message); }
    }
    document.getElementById('zd-validate').addEventListener('click', function () {
      try {
        const parsed = parse();
        const name = parsed && parsed.name;
        setResult('auth-status-allowed',
          name ? 'JSON OK; namespace name = "' + name + '"' : 'JSON OK; missing "name" field — server will reject');
      } catch (err) {
        setResult('auth-status-denied', err.message);
      }
    });
    document.getElementById('zd-submit').addEventListener('click', async function () {
      let body;
      try { body = parse(); }
      catch (err) { setResult('auth-status-denied', err.message); return; }
      try {
        await ctx.api.defineZanzibarNamespace(body);
        setResult('auth-status-allowed', 'Namespace "' + (body.name || '?') + '" defined.');
      } catch (err) {
        setResult('auth-status-denied', 'Error: ' + err.message);
      }
    });
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

  const BOOTSTRAP_TUPLES = [
    { object_type: 'auth',     object_id: 'system', relation: 'admin'  },
    { object_type: 'engine',   object_id: 'core',   relation: 'admin'  },
    { object_type: 'workflow', object_id: 'main',   relation: 'access' },
    { object_type: 'vault',    object_id: 'main',   relation: 'access' },
  ];

  function renderBootstrapForm() {
    const wrap = container.querySelector('#zb-wrap');
    let tuplesList = '';
    for (let i = 0; i < BOOTSTRAP_TUPLES.length; i++) {
      const t = BOOTSTRAP_TUPLES[i];
      tuplesList += '<li class="auth-mono">' +
        ctx.escapeHtml(t.object_type) + ':' + ctx.escapeHtml(t.object_id) +
        '#' + ctx.escapeHtml(t.relation) + '</li>';
    }
    wrap.innerHTML = '<h3>Bootstrap admin</h3>' +
      '<p class="auth-empty">Grant the four canonical admin tuples to a user. Requires the canonical namespaces (<code>auth</code>, <code>engine</code>, <code>workflow</code>, <code>vault</code>) to already be defined.</p>' +
      '<div class="auth-form">' +
        '<label for="zba-user">User</label>' +
        '<select id="zba-user"><option value="">Loading users…</option></select>' +
        '<div>Will write:</div>' +
        '<ul style="margin:4px 0 8px 18px;">' + tuplesList + '</ul>' +
        '<div class="auth-form-actions">' +
          '<button type="button" class="btn btn-primary" id="zba-go" disabled>Grant admin</button>' +
        '</div>' +
      '</div>' +
      '<div id="zba-result" style="margin-top:16px;"></div>';

    const sel = document.getElementById('zba-user');
    const go = document.getElementById('zba-go');
    function result(cls, msg) {
      document.getElementById('zba-result').innerHTML = '<p class="' + cls + '">' + msg + '</p>';
    }

    ctx.api.listUsers({ limit: 200 }).then(function (data) {
      const items = (data && data.items) || [];
      if (!items.length) {
        sel.innerHTML = '<option value="">— no users; create one in the Users tab first —</option>';
        return;
      }
      let opts = '<option value="">— pick a user —</option>';
      for (let i = 0; i < items.length; i++) {
        const u = items[i];
        const label = (u.email || u.id) + (u.display_name ? ' (' + u.display_name + ')' : '');
        opts += '<option value="' + ctx.escapeHtml(u.id) + '">' + ctx.escapeHtml(label) + '</option>';
      }
      sel.innerHTML = opts;
      sel.addEventListener('change', function () { go.disabled = !sel.value; });
    }).catch(function (err) {
      sel.innerHTML = '<option value="">error loading users</option>';
      result('auth-status-denied', 'Could not list users: ' + ctx.escapeHtml(err.message));
    });

    go.addEventListener('click', async function () {
      const userId = sel.value;
      if (!userId) return;
      go.disabled = true;
      const written = [];
      const failed = [];
      for (let i = 0; i < BOOTSTRAP_TUPLES.length; i++) {
        const t = BOOTSTRAP_TUPLES[i];
        const tuple = Object.assign({}, t, {
          subject_type: 'user',
          subject_id: userId,
          subject_rel: null,
        });
        try {
          await ctx.api.writeZanzibarTuple(tuple);
          written.push(t.object_type + ':' + t.object_id + '#' + t.relation);
        } catch (err) {
          failed.push(t.object_type + ':' + t.object_id + '#' + t.relation + ' — ' + err.message);
        }
      }
      let html = '';
      if (written.length) {
        html += '<p class="auth-status-allowed">Wrote ' + written.length + ' tuple(s) for user ' + ctx.escapeHtml(userId) + '.</p>';
        html += '<ul style="margin:4px 0 8px 18px;" class="auth-mono">';
        for (let i = 0; i < written.length; i++) html += '<li>' + ctx.escapeHtml(written[i]) + '</li>';
        html += '</ul>';
      }
      if (failed.length) {
        html += '<p class="auth-status-denied">Failed ' + failed.length + ' tuple(s):</p><ul>';
        for (let i = 0; i < failed.length; i++) html += '<li>' + ctx.escapeHtml(failed[i]) + '</li>';
        html += '</ul>';
      }
      document.getElementById('zba-result').innerHTML = html;
      go.disabled = false;
    });
  }

  if (typeof window !== 'undefined') {
    window.AssayAuthZanzibar = { render: render };
  }

  return { render: render };
})();
