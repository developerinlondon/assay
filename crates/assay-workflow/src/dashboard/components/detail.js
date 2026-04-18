/* Assay Workflow Dashboard - Detail Panel Component */

var AssayDetail = (function () {
  'use strict';

  let panel = null;
  let ctx = null;
  // Per-instance close handler. The side-panel flow collapses the panel
  // itself; the inline-row flow removes the expansion <tr>. Both invoke
  // whatever's registered here at the moment the ✕ button is clicked.
  let activeClose = null;

  function getPanel() {
    if (!panel) panel = document.getElementById('detail-panel');
    return panel;
  }

  /**
   * Render the workflow detail into a target element.
   *
   * @param {string} id — workflow id
   * @param {object} context — shared dashboard ctx (apiFetch, escapeHtml, …)
   * @param {object} [opts]
   * @param {HTMLElement} [opts.target] — element to render into (default:
   *                       the sidebar #detail-panel)
   * @param {function} [opts.onClose] — called when the user clicks the ✕;
   *                       default collapses the side panel
   */
  async function showDetail(id, context, opts) {
    ctx = context;
    var p = (opts && opts.target) || getPanel();
    activeClose = (opts && opts.onClose) || closeDetail;

    p.innerHTML = '<div class="detail-header"><h2>Loading...</h2>' +
      '<button class="detail-close" id="detail-close-btn">&times;</button></div>';
    if (p === getPanel()) p.classList.add('open');

    p.querySelector('#detail-close-btn').addEventListener('click', function () {
      activeClose && activeClose();
    });

    try {
      // /state returns 404 when the workflow hasn't written a snapshot
      // yet (the common case for workflows that don't use
      // ctx:register_query). Catch that per-promise so it doesn't bubble
      // up and kill the whole detail render.
      var statePromise = ctx
        .apiFetch('/workflows/' + encodeURIComponent(id) + '/state')
        .catch(function () { return null; });

      var [wf, events, children, state] = await Promise.all([
        ctx.apiFetch('/workflows/' + encodeURIComponent(id)),
        ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/events'),
        ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/children'),
        statePromise,
      ]);

      renderDetail(wf, events || [], children || [], state, p);
    } catch (err) {
      p.innerHTML =
        '<div class="detail-header"><h2>Error</h2>' +
        '<button class="detail-close" id="detail-close-btn">&times;</button></div>' +
        '<div class="detail-body"><div class="error-box">' + ctx.escapeHtml(err.message) + '</div></div>';
      p.querySelector('#detail-close-btn').addEventListener('click', function () {
        activeClose && activeClose();
      });
    }
  }

  function renderDetail(wf, events, children, state, targetEl) {
    var p = targetEl || getPanel();
    var status = (wf.status || 'PENDING').toUpperCase();
    var terminal = ctx.isTerminal(status);

    var html =
      '<div class="detail-header">' +
        // Full workflow id — the detail view has the width for it, and
        // operators consulting this panel are usually trying to read or
        // copy the id in full. `word-break: break-all` in .detail-header
        // h2 (see style.css) keeps long ids wrapping cleanly instead of
        // bursting out of the container.
        '<h2 class="detail-id" title="' + ctx.escapeHtml(wf.id) + '">' +
          ctx.escapeHtml(wf.id) + '</h2>' +
        '<button class="detail-close" id="detail-close-btn">&times;</button>' +
      '</div>' +
      '<div class="detail-body">';

    // Two-column grid: fixed-width left column carries the run's
    // identity card (status badge, meta items stacked, actions) and
    // stays visible regardless of which tab is selected. Right column
    // gets the rest of the horizontal space for tab content, which is
    // the variable-height material. On narrow viewports the grid
    // collapses to a single column (see .detail-grid in style.css).
    html += '<div class="detail-grid">';

    // ── LEFT column: core info + actions ───────────────
    html += '<div class="detail-core">';

    html += '<div class="detail-core-status">' +
      '<span class="badge ' + ctx.badgeClass(status) + '">' + status + '</span>' +
      '</div>';

    html +=
      '<dl class="detail-core-meta">' +
        coreMeta('Type', ctx.escapeHtml(wf.workflow_type || '-')) +
        coreMeta('Namespace', ctx.escapeHtml(wf.namespace || '-')) +
        coreMeta('Queue', ctx.escapeHtml(wf.task_queue || '-')) +
        coreMeta('Run ID',
          '<span class="mono meta-id">' + ctx.escapeHtml(wf.run_id || '-') + '</span>') +
        coreMeta('Created', ctx.formatTime(wf.created_at)) +
        coreMeta('Claimed By', ctx.escapeHtml(wf.claimed_by || '-')) +
        (wf.completed_at
          ? coreMeta('Completed', ctx.formatTime(wf.completed_at))
          : '') +
      '</dl>';

    // Actions sit at the bottom of the left column, below the meta.
    var idAttr = ctx.escapeHtml(wf.id);
    html += '<div class="detail-core-actions">';
    if (!terminal) {
      html +=
        '<button class="btn-action btn-signal-detail" data-id="' + idAttr + '">Send signal</button>' +
        '<button class="btn-action btn-cancel-detail" data-id="' + idAttr + '">Cancel</button>' +
        '<button class="btn-action btn-action-danger btn-terminate-detail" data-id="' + idAttr + '">Terminate</button>' +
        '<button class="btn-action btn-continue-detail" data-id="' + idAttr + '">Continue as new</button>';
    } else {
      html +=
        '<button class="btn-action btn-continue-detail" data-id="' + idAttr + '" title="Start a fresh run with the same type + queue">Continue as new</button>';
    }
    html += '</div>'; // /detail-core-actions

    html += '</div>'; // /detail-core (left column closed)

    // ── RIGHT column: tabs + panel content ─────────────
    html += '<div class="detail-tabs-col">';

    // Tabs — Overview / State / Events / Children / Attributes. Variable-
    // height content lives behind tabs so the left column (identity +
    // actions) stays compact and scannable regardless of how much data
    // a run has accumulated. Tabs that have no content (no state
    // snapshot, zero children, no search attrs) are dimmed rather than
    // hidden so operators have consistent visual anchors across runs.

    var tabs = [
      {
        id: 'overview',
        label: 'Overview',
        count: null,
        build: function () {
          var body = '';
          if (wf.input) {
            body += '<h4 class="detail-subhead">Input</h4>' +
              '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.input)) + '</div>';
          }
          if (wf.result) {
            body += '<h4 class="detail-subhead">Result</h4>' +
              '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.result)) + '</div>';
          }
          if (wf.error) {
            body += '<h4 class="detail-subhead" style="color: var(--red);">Error</h4>' +
              '<div class="error-box">' + ctx.escapeHtml(wf.error) + '</div>';
          }
          if (!body) {
            body = '<p class="detail-muted">No input, result, or error recorded.</p>';
          }
          return body;
        },
      },
      {
        id: 'state',
        label: 'State',
        empty: !(state && state.state !== undefined && state.state !== null),
        build: function () {
          if (!state || state.state === undefined || state.state === null) {
            return '<p class="detail-muted">' +
              'No live state snapshot. This workflow did not call <code>ctx:register_query</code>.' +
              '</p>';
          }
          var stateJson = typeof state.state === 'string'
            ? state.state
            : JSON.stringify(state.state, null, 2);
          return '<div class="json-viewer">' + ctx.escapeHtml(stateJson) + '</div>' +
            '<p class="detail-muted" style="margin-top: 6px;">' +
              'Snapshot at event seq ' + (state.event_seq || '?') +
              (state.created_at ? ' — ' + ctx.formatTime(state.created_at) : '') +
            '</p>';
        },
      },
      {
        id: 'events',
        label: 'Events',
        count: events.length,
        build: function () {
          if (!events.length) return '<p class="detail-muted">No events recorded.</p>';
          var out = '<div class="event-timeline">';
          for (var i = 0; i < events.length; i++) {
            var evt = events[i];
            out +=
              '<div class="event-item" data-idx="' + i + '">' +
                '<div class="event-header">' +
                  '<span class="event-type">' + ctx.escapeHtml(evt.event_type) + '</span>' +
                  '<span class="event-time">#' + evt.seq + ' - ' + ctx.formatTime(evt.timestamp) + '</span>' +
                '</div>' +
                '<div class="event-payload" id="evt-payload-' + i + '">' +
                  (evt.payload
                    ? '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(evt.payload)) + '</div>'
                    : '<span style="color: var(--text-muted); font-size: 12px;">No payload</span>') +
                '</div>' +
              '</div>';
          }
          return out + '</div>';
        },
      },
      {
        id: 'children',
        label: 'Children',
        count: children.length,
        empty: children.length === 0,
        build: function () {
          if (!children.length) return '<p class="detail-muted">No child workflows.</p>';
          var out = '<table class="data-table"><thead><tr>' +
            '<th>ID</th><th>Type</th><th>Status</th></tr></thead><tbody>';
          for (var j = 0; j < children.length; j++) {
            var child = children[j];
            var cs = (child.status || 'PENDING').toUpperCase();
            out +=
              '<tr>' +
                '<td><a href="#" class="clickable child-link mono" data-id="' + ctx.escapeHtml(child.id) + '" title="' + ctx.escapeHtml(child.id) + '">' +
                  ctx.escapeHtml(ctx.truncate(child.id, 28)) + '</a></td>' +
                '<td>' + ctx.escapeHtml(child.workflow_type || '-') + '</td>' +
                '<td><span class="badge ' + ctx.badgeClass(cs) + '">' + cs + '</span></td>' +
              '</tr>';
          }
          return out + '</tbody></table>';
        },
      },
      {
        id: 'attrs',
        label: 'Attributes',
        empty: !wf.search_attributes,
        build: function () {
          if (!wf.search_attributes) {
            return '<p class="detail-muted">No search attributes set on this run.</p>';
          }
          return '<div class="json-viewer">' +
            ctx.escapeHtml(ctx.formatJson(wf.search_attributes)) +
            '</div>';
        },
      },
    ];

    html += '<div class="detail-tabs">';
    html += '<div class="detail-tab-nav" role="tablist">';
    for (var t = 0; t < tabs.length; t++) {
      var tab = tabs[t];
      var active = t === 0 ? ' active' : '';
      var dim = tab.empty ? ' dim' : '';
      var label = tab.label +
        (tab.count != null ? ' <span class="tab-count">(' + tab.count + ')</span>' : '');
      html +=
        '<button class="detail-tab' + active + dim +
        '" data-tab="' + tab.id + '" role="tab">' +
          label +
        '</button>';
    }
    html += '</div>'; // /detail-tab-nav
    html += '<div class="detail-tab-panels">';
    for (var u = 0; u < tabs.length; u++) {
      var active2 = u === 0 ? ' active' : '';
      html +=
        '<div class="detail-tab-panel' + active2 + '" data-tab="' + tabs[u].id + '" role="tabpanel">' +
          tabs[u].build() +
        '</div>';
    }
    html += '</div>'; // /detail-tab-panels
    html += '</div>'; // /detail-tabs

    html += '</div>'; // /detail-tabs-col (right column closed)
    html += '</div>'; // /detail-grid (two-column grid closed)

    html += '</div>'; // /detail-body
    p.innerHTML = html;

    // Wire up event delegation
    p.addEventListener('click', handlePanelClick);
  }

  function handlePanelClick(e) {
    // Close button
    if (e.target.closest('#detail-close-btn') || e.target.closest('.detail-close')) {
      (activeClose || closeDetail)();
      return;
    }

    // Tab switch — click on a .detail-tab swaps the active tab + panel
    // within the same detail container. Uses the closest .detail-tabs
    // ancestor so multiple detail blocks on the page (e.g. the side
    // panel + an inline-row expansion) don't cross-trigger each other.
    var tabBtn = e.target.closest('.detail-tab');
    if (tabBtn) {
      e.preventDefault();
      var container = tabBtn.closest('.detail-tabs');
      if (!container) return;
      var id = tabBtn.dataset.tab;
      var tabs = container.querySelectorAll('.detail-tab');
      for (var i = 0; i < tabs.length; i++) tabs[i].classList.remove('active');
      tabBtn.classList.add('active');
      var panels = container.querySelectorAll('.detail-tab-panel');
      for (var j = 0; j < panels.length; j++) {
        panels[j].classList.toggle('active', panels[j].dataset.tab === id);
      }
      return;
    }

    // Event item toggle
    var evtItem = e.target.closest('.event-item');
    if (evtItem) {
      var idx = evtItem.dataset.idx;
      var payload = document.getElementById('evt-payload-' + idx);
      if (payload) payload.classList.toggle('open');
      return;
    }

    // Child link
    var childLink = e.target.closest('.child-link');
    if (childLink) {
      e.preventDefault();
      showDetail(childLink.dataset.id, ctx);
      return;
    }

    // Signal button
    var sigBtn = e.target.closest('.btn-signal-detail');
    if (sigBtn) {
      handleSignal(sigBtn.dataset.id);
      return;
    }

    // Cancel button
    var canBtn = e.target.closest('.btn-cancel-detail');
    if (canBtn) {
      handleCancel(canBtn.dataset.id);
      return;
    }

    // Terminate button
    var termBtn = e.target.closest('.btn-terminate-detail');
    if (termBtn) {
      handleTerminate(termBtn.dataset.id);
      return;
    }

    // Continue-as-new button
    var contBtn = e.target.closest('.btn-continue-detail');
    if (contBtn) {
      handleContinueAsNew(contBtn.dataset.id);
    }
  }

  async function handleSignal(id) {
    var name = prompt('Signal name:');
    if (!name) return;
    var payloadStr = prompt('Signal payload (JSON, or leave empty):', '');
    var payload = null;
    if (payloadStr) {
      try { payload = JSON.parse(payloadStr); } catch (_) { payload = payloadStr; }
    }
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/signal/' + encodeURIComponent(name), {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ payload: payload }),
      });
      ctx.toast("Signal '" + name + "' sent", 'success');
      showDetail(id, ctx);
    } catch (err) {
      ctx.toast('Signal failed: ' + err.message, 'error');
    }
  }

  async function handleCancel(id) {
    if (!confirm('Cancel workflow ' + id + '?')) return;
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/cancel', { method: 'POST' });
      ctx.toast('Cancel requested', 'success');
      showDetail(id, ctx);
    } catch (err) {
      ctx.toast('Cancel failed: ' + err.message, 'error');
    }
  }

  async function handleTerminate(id) {
    var reason = prompt(
      'Terminate workflow ' + id + '?\n\nReason (optional):',
      ''
    );
    if (reason === null) return;
    var body = reason ? { reason: reason } : {};
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/terminate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      ctx.toast('Terminated', 'success');
      showDetail(id, ctx);
    } catch (err) {
      ctx.toast('Terminate failed: ' + err.message, 'error');
    }
  }

  async function handleContinueAsNew(id) {
    var inputStr = prompt(
      'Close out ' + id + ' and start a fresh run with the same type + queue.\n\n' +
      'New input (JSON, optional):',
      ''
    );
    if (inputStr === null) return;
    var body = {};
    if (inputStr && inputStr.trim()) {
      try {
        body.input = JSON.parse(inputStr);
      } catch (err) {
        ctx.toast('Input must be valid JSON', 'error');
        return;
      }
    }
    try {
      var newRun = await ctx.apiFetch(
        '/workflows/' + encodeURIComponent(id) + '/continue-as-new',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify(body),
        }
      );
      ctx.toast('New run: ' + (newRun && newRun.workflow_id || 'unknown'), 'success');
      if (newRun && newRun.workflow_id) {
        showDetail(newRun.workflow_id, ctx);
      } else {
        closeDetail();
      }
      if (ctx.refreshCurrentView) ctx.refreshCurrentView();
    } catch (err) {
      ctx.toast('Continue-as-new failed: ' + err.message, 'error');
    }
  }


  // Left-column meta row: `<dt>Label</dt><dd>Value</dd>`. Using a
  // semantic definition list so each row pairs cleanly without needing
  // flex plumbing, and screen readers announce the label/value
  // relationship.
  function coreMeta(label, value) {
    return '<dt>' + label + '</dt><dd>' + value + '</dd>';
  }

  function closeDetail() {
    var p = getPanel();
    p.classList.remove('open');
    p.removeEventListener('click', handlePanelClick);
    setTimeout(function () { p.innerHTML = ''; }, 300);
  }

  return { showDetail: showDetail, closeDetail: closeDetail };
})();
