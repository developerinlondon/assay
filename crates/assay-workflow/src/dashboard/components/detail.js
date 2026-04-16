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
      var [wf, events, children] = await Promise.all([
        ctx.apiFetch('/workflows/' + encodeURIComponent(id)),
        ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/events'),
        ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/children'),
      ]);

      renderDetail(wf, events || [], children || []);
    } catch (err) {
      p.innerHTML =
        '<div class="detail-header"><h2>Error</h2>' +
        '<button class="detail-close" id="detail-close-btn">&times;</button></div>' +
        '<div class="detail-body"><div class="error-box">' + ctx.escapeHtml(err.message) + '</div></div>';
      p.querySelector('#detail-close-btn').addEventListener('click', closeDetail);
    }
  }

  function renderDetail(wf, events, children) {
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
    if (!terminal) {
      html +=
        '<div style="margin-bottom: 16px;">' +
          '<button class="btn btn-sm btn-signal-detail" data-id="' + ctx.escapeHtml(wf.id) + '">Signal</button> ' +
          '<button class="btn btn-sm btn-danger btn-cancel-detail" data-id="' + ctx.escapeHtml(wf.id) + '">Cancel</button>' +
        '</div>';
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
      showDetail(id, ctx);
    } catch (err) {
      alert('Signal failed: ' + err.message);
    }
  }

  async function handleCancel(id) {
    if (!confirm('Cancel workflow ' + id + '?')) return;
    try {
      await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/cancel', { method: 'POST' });
      showDetail(id, ctx);
    } catch (err) {
      alert('Cancel failed: ' + err.message);
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
