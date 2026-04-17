/* Assay Workflow Dashboard - Detail Panel Component */

var AssayDetail = (function () {
  'use strict';

  let panel = null;
  let ctx = null;

  function getPanel() {
    if (!panel) panel = document.getElementById('detail-panel');
    return panel;
  }

  async function showDetail(id, context) {
    ctx = context;
    var p = getPanel();
    p.innerHTML = '<div class="detail-header"><h2>Loading...</h2>' +
      '<button class="detail-close" id="detail-close-btn">&times;</button></div>';
    p.classList.add('open');

    p.querySelector('#detail-close-btn').addEventListener('click', closeDetail);

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

      renderDetail(wf, events || [], children || [], state);
    } catch (err) {
      p.innerHTML =
        '<div class="detail-header"><h2>Error</h2>' +
        '<button class="detail-close" id="detail-close-btn">&times;</button></div>' +
        '<div class="detail-body"><div class="error-box">' + ctx.escapeHtml(err.message) + '</div></div>';
      p.querySelector('#detail-close-btn').addEventListener('click', closeDetail);
    }
  }

  function renderDetail(wf, events, children, state) {
    var p = getPanel();
    var status = (wf.status || 'PENDING').toUpperCase();
    var terminal = ctx.isTerminal(status);

    var html =
      '<div class="detail-header">' +
        '<h2>' + ctx.escapeHtml(ctx.truncate(wf.id, 40)) + '</h2>' +
        '<button class="detail-close" id="detail-close-btn">&times;</button>' +
      '</div>' +
      '<div class="detail-body">';

    // Metadata grid
    html +=
      '<div class="meta-grid">' +
        metaItem('Status', '<span class="badge ' + ctx.badgeClass(status) + '">' + status + '</span>') +
        metaItem('Type', ctx.escapeHtml(wf.workflow_type || '-')) +
        metaItem('Namespace', ctx.escapeHtml(wf.namespace || '-')) +
        metaItem('Queue', ctx.escapeHtml(wf.task_queue || '-')) +
        metaItem('Run ID', '<span class="mono">' + ctx.escapeHtml(ctx.truncate(wf.run_id, 24)) + '</span>') +
        metaItem('Created', ctx.formatTime(wf.created_at)) +
        metaItem('Claimed By', ctx.escapeHtml(wf.claimed_by || '-')) +
        metaItem('Completed', wf.completed_at ? ctx.formatTime(wf.completed_at) : '-') +
      '</div>';

    // Actions
    var idAttr = ctx.escapeHtml(wf.id);
    html += '<div class="action-row" style="margin-bottom: 16px;">';
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
    html += '</div>';

    // Live state snapshot (populated by ctx:register_query handlers in the
    // workflow code). Only shown if a snapshot exists — workflows that
    // don't register queries just won't have this section.
    if (state && state.state !== undefined && state.state !== null) {
      var stateJson = typeof state.state === 'string'
        ? state.state
        : JSON.stringify(state.state, null, 2);
      html +=
        '<h3 style="margin-bottom: 8px;">Live state</h3>' +
        '<div class="json-viewer">' + ctx.escapeHtml(stateJson) + '</div>' +
        '<p style="color: var(--text-muted); font-size: 12px; margin-top: 4px;">' +
          'Snapshot at event seq ' + (state.event_seq || '?') +
          (state.created_at ? ' — ' + ctx.formatTime(state.created_at) : '') +
        '</p>';
    }

    // Input
    if (wf.input) {
      html += collapsible('Input', '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.input)) + '</div>');
    }

    // Result
    if (wf.result) {
      html += collapsible('Result', '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.result)) + '</div>');
    }

    // Error
    if (wf.error) {
      html += '<h3 style="margin-bottom: 8px; color: var(--red);">Error</h3>' +
        '<div class="error-box">' + ctx.escapeHtml(wf.error) + '</div>';
    }

    // Event timeline
    html += '<h3 style="margin-bottom: 12px;">Events (' + events.length + ')</h3>';
    if (events.length > 0) {
      html += '<div class="event-timeline">';
      for (var i = 0; i < events.length; i++) {
        var evt = events[i];
        html +=
          '<div class="event-item" data-idx="' + i + '">' +
            '<div class="event-header">' +
              '<span class="event-type">' + ctx.escapeHtml(evt.event_type) + '</span>' +
              '<span class="event-time">#' + evt.seq + ' - ' + ctx.formatTime(evt.timestamp) + '</span>' +
            '</div>' +
            '<div class="event-payload" id="evt-payload-' + i + '">';

        if (evt.payload) {
          html += '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(evt.payload)) + '</div>';
        } else {
          html += '<span style="color: var(--text-muted); font-size: 12px;">No payload</span>';
        }

        html += '</div></div>';
      }
      html += '</div>';
    } else {
      html += '<p style="color: var(--text-muted);">No events recorded</p>';
    }

    // Children
    if (children.length > 0) {
      html += '<h3 style="margin: 20px 0 12px;">Child Workflows (' + children.length + ')</h3>';
      html += '<table class="data-table"><thead><tr>' +
        '<th>ID</th><th>Type</th><th>Status</th>' +
        '</tr></thead><tbody>';
      for (var j = 0; j < children.length; j++) {
        var child = children[j];
        var cs = (child.status || 'PENDING').toUpperCase();
        html +=
          '<tr>' +
            '<td><a href="#" class="clickable child-link mono" data-id="' + ctx.escapeHtml(child.id) + '">' +
              ctx.escapeHtml(ctx.truncate(child.id, 28)) + '</a></td>' +
            '<td>' + ctx.escapeHtml(child.workflow_type || '-') + '</td>' +
            '<td><span class="badge ' + ctx.badgeClass(cs) + '">' + cs + '</span></td>' +
          '</tr>';
      }
      html += '</tbody></table>';
    }

    html += '</div>';
    p.innerHTML = html;

    // Wire up event delegation
    p.addEventListener('click', handlePanelClick);
  }

  function handlePanelClick(e) {
    // Close button
    if (e.target.closest('#detail-close-btn') || e.target.closest('.detail-close')) {
      closeDetail();
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

  function collapsible(title, content) {
    var id = 'coll-' + title.toLowerCase().replace(/\s+/g, '-');
    return '<div class="collapsible-header" onclick="document.getElementById(\'' + id + '\').classList.toggle(\'open\')">' +
      '&#9654; ' + title +
      '</div>' +
      '<div class="collapsible-content" id="' + id + '">' + content + '</div>';
  }

  function metaItem(label, value) {
    return '<div class="meta-item"><label>' + label + '</label><span>' + value + '</span></div>';
  }

  function closeDetail() {
    var p = getPanel();
    p.classList.remove('open');
    p.removeEventListener('click', handlePanelClick);
    setTimeout(function () { p.innerHTML = ''; }, 300);
  }

  return { showDetail: showDetail, closeDetail: closeDetail };
})();
