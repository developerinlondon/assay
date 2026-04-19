/* Assay Workflow Dashboard - Detail Panel Component */

var AssayDetail = (function () {
  'use strict';

  let panel = null;
  let ctx = null;
  // Per-instance close handler. The side-panel flow collapses the panel
  // itself; the inline-row flow removes the expansion <tr>. Both invoke
  // whatever's registered here at the moment the ✕ button is clicked.
  let activeClose = null;
  // closeDetail queues a 300ms innerHTML clear (after the slide-out
  // animation). If a new showDetail starts before that clear fires,
  // we'd wipe the freshly-rendered content. Track the pending timeout
  // so showDetail can cancel it.
  let pendingClearTimeout = null;
  // Single live-tail poller. The Pipeline tab polls
  // /workflows/{id}/state/pipeline_state every 1s while the tab is open
  // and the workflow is RUNNING; only one detail panel renders at a
  // time so one poller handle is enough.
  let activePoller = null;

  // Step status → glyph + class. Kept tight so the same map drives
  // initial render and live-update DOM mutations. Six canonical
  // statuses; document the convention in docs/modules/workflow.md
  // before adding more.
  //
  // `running` uses a proper SVG spinner instead of a text glyph because
  // rotating an asymmetric unicode arrow inside the circle looked
  // janky — a symmetric ring with a gap rotates cleanly. Other states
  // are text glyphs (no animation needed).
  const RUNNING_SPINNER_SVG =
    '<svg class="step-spinner" viewBox="0 0 24 24" width="20" height="20" ' +
        'fill="none" stroke="currentColor" stroke-width="2.5" ' +
        'stroke-linecap="round" aria-hidden="true">' +
      '<path d="M 12 2 a 10 10 0 0 1 10 10" />' +
    '</svg>';
  const STEP_STATE = {
    waiting:   { glyph: '\u25CB', cls: 'waiting' },   // ○ outlined dim
    running:   { glyph: RUNNING_SPINNER_SVG, cls: 'running', isHtml: true },
    done:      { glyph: '\u2713', cls: 'done' },      // ✓ filled green
    failed:    { glyph: '\u2715', cls: 'failed' },    // ✕ filled red
    cancelled: { glyph: '\u2298', cls: 'cancelled' }, // ⊘ no-entry orange
    skipped:   { glyph: '\u00B7', cls: 'skipped' },   // · muted dot — never ran
  };

  function stepView(status) {
    return STEP_STATE[(status || 'waiting').toLowerCase()] || STEP_STATE.waiting;
  }

  // Connector fill class derived from the state of the two steps it
  // joins. `done → done` is fully filled; `done → running` is partial;
  // anything leading into a `skipped` step is muted (the workflow
  // ended before reaching it).
  function connectorFill(prev, next) {
    var p = (prev || '').toLowerCase();
    var n = (next || '').toLowerCase();
    if (n === 'skipped') return 'muted';
    if (p === 'done' && (n === 'done' || n === 'failed' || n === 'cancelled')) return 'full';
    if (p === 'done' && n === 'running') return 'partial';
    if (p === 'done') return 'partial';
    return 'muted';
  }

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

    // Cancel any pending innerHTML clear from a prior closeDetail —
    // otherwise the freshly-rendered content would be wiped 300ms in.
    if (pendingClearTimeout) {
      clearTimeout(pendingClearTimeout);
      pendingClearTimeout = null;
    }

    p.innerHTML = '<div class="detail-header"><h2>Loading...</h2>' +
      '<button class="detail-close" id="detail-close-btn">&times;</button></div>';
    if (p === getPanel()) {
      p.classList.add('open');
      p.dataset.workflowId = id;
      // Mirror to the URL hash so F5 / share-link restore lands here.
      if (window.AssayApp && window.AssayApp.setOpenWorkflow) {
        window.AssayApp.setOpenWorkflow(id);
      }
    }

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

    // Every identity/config field shown in the detail block is also
    // surfaced on the row above (id, type, status, queue, created) or
    // in the namespace selector. The only field the row can't carry is
    // `completed_at` — workflow list rows show "Created X ago" but not
    // a completion timestamp. Put that on a slim meta row above the
    // tabs so terminal runs get their one missing piece; no meta row
    // at all for in-flight runs (nothing new to say).
    //
    // The action buttons (Signal / Cancel / Terminate / Continue-as-
    // new) used to live on a toolbar here too; v0.12.0 dropped them
    // because they duplicate the row's Action column. The row now
    // shows them as icons with tooltips, so the detail view stays
    // identity-only and the tabs get all the vertical space.
    if (wf.completed_at) {
      html += '<div class="detail-meta-line">' +
        '<span class="detail-meta-label">Completed</span> ' +
        '<span class="detail-meta-value">' + ctx.formatTime(wf.completed_at) + '</span>' +
        '</div>';
    }

    // Tabs — [Pipeline] / Overview / State / Events / Children / Attributes.
    // Pipeline tab is added at the front (and default-selected) only when the
    // workflow has registered a `pipeline_state` query that includes a
    // `steps[]` array — see docs/modules/workflow.md "Pipeline tab convention".
    // Variable-height content lives behind tabs so the meta line + actions
    // toolbar stay compact regardless of how much data a run has accumulated.
    // Tabs with no content are dimmed rather than hidden so operators have
    // consistent visual anchors across runs.

    var pipeline = extractPipelineState(state);
    if (pipeline && terminal) applyTerminalOverlay(pipeline, status);

    var tabs = [
      {
        id: 'overview',
        label: 'Overview',
        count: null,
        build: function () { return renderOverviewHtml(wf); },
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
          // Master-detail layout: list on the left, selected event's
          // payload on the right. Much easier to scan than stacked
          // expandables and mirrors how operators actually read event
          // history (pick one → inspect, pick another → inspect).
          var out = '<div class="event-split">';
          out += '<div class="event-list" role="tablist">';
          for (var i = 0; i < events.length; i++) {
            var evt = events[i];
            var activeCls = i === 0 ? ' active' : '';
            // Exact wall-clock time on the left list so operators can
            // line events up against external logs ("what happened at
            // 14:32:08?"). The "N seconds ago" relative form stays on
            // the right-side detail panel for at-a-glance freshness.
            out +=
              '<button type="button" class="event-list-item' + activeCls + '" data-idx="' + i + '" role="tab">' +
                '<span class="event-seq">#' + evt.seq + '</span>' +
                '<span class="event-type">' + ctx.escapeHtml(evt.event_type) + '</span>' +
                '<span class="event-time">' + ctx.formatExactTime(evt.timestamp) + '</span>' +
              '</button>';
          }
          out += '</div>'; // /event-list
          out += '<div class="event-detail">';
          for (var j = 0; j < events.length; j++) {
            var e = events[j];
            var show = j === 0 ? '' : ' hidden';
            out += '<div class="event-detail-panel' + show + '" data-idx="' + j + '">' +
              '<div class="event-detail-head">' +
                '<span class="event-detail-type">' + ctx.escapeHtml(e.event_type) + '</span>' +
                '<span class="event-detail-meta">#' + e.seq + ' — ' +
                  ctx.formatExactTime(e.timestamp) + ' (' + ctx.formatTime(e.timestamp) + ')' +
                '</span>' +
              '</div>' +
              (e.payload
                ? '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(e.payload)) + '</div>'
                : '<p class="detail-muted">No payload for this event.</p>') +
            '</div>';
          }
          out += '</div>'; // /event-detail
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

    // Steps tab is prepended when present and becomes the default-active
    // tab — that's the highest-density "what's happening right now" view a
    // staged workflow has, so showing it first matches the operator's most
    // common intent (open the run → see how far it's got). Internal CSS
    // class names keep `.pipeline-*` for back-compat with consumer
    // whitelabel themes that might reference them; the visible label
    // follows our chosen "steps" vocabulary (see docs/modules/workflow.md).
    if (pipeline) {
      tabs.unshift({
        id: 'pipeline',
        label: 'Steps',
        count: pipeline.steps.length,
        build: function () { return renderPipelineHtml(wf, pipeline); },
      });
    }

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

    html += '</div>'; // /detail-body
    p.innerHTML = html;

    // Wire up event delegation
    p.addEventListener('click', handlePanelClick);

    // Auto-advance focus on initial render too — terminal workflows
    // never poll, so this is the only chance to highlight the
    // last-relevant step (the failed one for FAILED runs, the final
    // step for COMPLETED runs).
    if (pipeline) {
      var pipelineRoot = p.querySelector('.pipeline-tab');
      if (pipelineRoot) autoAdvanceFocus(pipelineRoot, pipeline);
      startCountdownTicker();
    }

    // Stop any prior poll first — switching from one workflow to another
    // would otherwise leak the previous interval. Kick off a new poller
    // for the Pipeline tab if it's present and the workflow is still
    // running. The poller handles its own teardown when the workflow
    // goes terminal.
    stopPoller();
    if (pipeline && !terminal) {
      startPipelinePoller(wf.id, p);
    }
  }

  function handlePanelClick(e) {
    // Close button
    if (e.target.closest('#detail-close-btn') || e.target.closest('.detail-close')) {
      (activeClose || closeDetail)();
      return;
    }

    // Cross-link — navigate to another view's detail (e.g. clicking
    // "Claimed by" in Overview switches to Workers + opens that
    // worker). Same handler shape as workers.js / queues.js so the
    // affordance works identically across all three detail panels.
    var cross = e.target.closest('.cross-link[data-nav]');
    if (cross) {
      e.preventDefault();
      e.stopPropagation();
      if (ctx && ctx.navigate) ctx.navigate(cross.dataset.nav, cross.dataset.id);
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
      // Pause poller when leaving Pipeline; resume when returning, but only
      // if the workflow is still RUNNING (terminal runs don't need the
      // 1Hz refresh). The poll endpoint is cheap, but idle traffic on a
      // bunch of completed runs would still be wasteful.
      if (id !== 'pipeline') {
        stopPoller();
      } else if (activePoller && activePoller.id) {
        // Already polling for this workflow, keep going.
      } else {
        var pipelineEl = container.querySelector('.pipeline-tab');
        if (pipelineEl && pipelineEl.dataset.terminal !== 'true') {
          startPipelinePoller(pipelineEl.dataset.workflowId, container.closest('.detail-body, [id]') || document);
        }
      }
      return;
    }

    // Pipeline step circle — clicking toggles a per-step log filter so
    // operators can drill into one slow step's lines without scrolling
    // the full timeline. Click again to clear the filter. Skip when the
    // click originated on an action button or its container — those are
    // their own handlers (step_action signal POST below) and bubbling
    // through the filter toggle would eat the click.
    var stepEl = e.target.closest('.pipeline-step');
    if (stepEl && !e.target.closest('.pipeline-step-action, .step-actions')) {
      var pipelineRoot = stepEl.closest('.pipeline-tab');
      if (pipelineRoot) {
        var alreadySelected = stepEl.classList.contains('selected');
        var allSteps = pipelineRoot.querySelectorAll('.pipeline-step');
        for (var s = 0; s < allSteps.length; s++) allSteps[s].classList.remove('selected');
        if (alreadySelected) {
          // Clearing manual selection — release auto-advance so the
          // next snapshot poll re-focuses on whatever's running.
          delete pipelineRoot.dataset.filterStep;
          delete pipelineRoot.dataset.userSelected;
        } else {
          stepEl.classList.add('selected');
          pipelineRoot.dataset.filterStep = stepEl.dataset.stepIdx;
          // Stick on this choice — auto-advance backs off until the
          // user clicks the same step again to release.
          pipelineRoot.dataset.userSelected = 'true';
        }
        applyLogFilter(pipelineRoot);
      }
      return;
    }

    // Pipeline scroll-lock toggle — operators reading mid-log don't want
    // the auto-scroll yanking them back to the bottom every second.
    // Toggle disables it; the indicator shows the current state.
    var scrollBtn = e.target.closest('.pipeline-scroll-lock');
    if (scrollBtn) {
      e.preventDefault();
      var pr = scrollBtn.closest('.pipeline-tab');
      if (pr) {
        var locked = pr.dataset.scrollLock === 'true';
        pr.dataset.scrollLock = locked ? 'false' : 'true';
        scrollBtn.textContent = locked ? '\u25BE Auto-scroll' : '\u25B8 Locked';
        if (!locked) {
          // Switching to "Locked" — leave the log where it is.
        } else {
          // Switching back to auto-scroll — jump to bottom now.
          var logEl = pr.querySelector('.pipeline-log');
          if (logEl) logEl.scrollTop = logEl.scrollHeight;
        }
      }
      return;
    }

    // Pipeline step action — buttons rendered next to each step that
    // exposes an `actions[]` array. Click → POST a `step_action` signal
    // to AWE; the workflow handler reads it and decides what to do.
    // Pure routing: AWE never knows what "approve" means, the workflow
    // does.
    var actBtn = e.target.closest('.pipeline-step-action');
    if (actBtn) {
      e.preventDefault();
      handleStepAction(
        actBtn.dataset.workflowId,
        actBtn.dataset.stepName,
        actBtn.dataset.action
      );
      return;
    }

    // Event list item — master-detail pattern. Clicking a row on
    // the left selects that event and shows its payload on the right.
    var evtItem = e.target.closest('.event-list-item');
    if (evtItem) {
      var idx = evtItem.dataset.idx;
      var eventsRoot = evtItem.closest('.event-split');
      if (eventsRoot) {
        var items = eventsRoot.querySelectorAll('.event-list-item');
        for (var ii = 0; ii < items.length; ii++) items[ii].classList.remove('active');
        evtItem.classList.add('active');
        var panels = eventsRoot.querySelectorAll('.event-detail-panel');
        for (var pi = 0; pi < panels.length; pi++) {
          panels[pi].toggleAttribute('hidden', panels[pi].dataset.idx !== idx);
        }
      }
      return;
    }

    // Child link
    var childLink = e.target.closest('.child-link');
    if (childLink) {
      e.preventDefault();
      showDetail(childLink.dataset.id, ctx);
      return;
    }

    // The four workflow actions (Signal / Cancel / Terminate /
    // Continue-as-new) used to render as buttons inside the detail
    // toolbar; v0.12.0 dropped the toolbar (the row's Action column
    // already exposes the same set as icons + tooltips). The handler
    // logic moved to AssayActions for reuse across views.
  }

  function closeDetail() {
    stopPoller();
    var p = getPanel();
    p.classList.remove('open');
    delete p.dataset.workflowId;
    if (window.AssayApp && window.AssayApp.setOpenWorkflow) {
      window.AssayApp.setOpenWorkflow(null);
    }
    p.removeEventListener('click', handlePanelClick);
    if (pendingClearTimeout) clearTimeout(pendingClearTimeout);
    pendingClearTimeout = setTimeout(function () {
      p.innerHTML = '';
      pendingClearTimeout = null;
    }, 300);
  }

  // Render the Overview tab — the executive summary of a workflow
  // run. A slim meta strip at the top with run-level context
  // (duration, run id, claimed worker) followed by whatever input/
  // result/error/search-attributes the run actually carries. Empty
  // sections are skipped — a pending run with `input = {}` doesn't
  // show an "Input: {}" line that's just noise.
  function renderOverviewHtml(wf) {
    var parts = [];
    var meta = [];

    meta.push(['Run ID', '<span class="mono">' + ctx.escapeHtml(wf.run_id || '-') + '</span>']);

    // Duration: elapsed from created_at to completed_at (or now).
    if (wf.created_at) {
      var end = wf.completed_at || (Date.now() / 1000);
      var dur = Math.max(0, end - wf.created_at);
      meta.push(['Duration', formatDuration(dur) +
        (wf.completed_at ? '' : ' (still running)')]);
    }

    if (wf.claimed_by) {
      var cb = ctx.escapeHtml(wf.claimed_by);
      meta.push(['Claimed by',
        '<a href="#" class="cross-link mono" data-nav="worker" data-id="' + cb + '">' + cb + '</a>']);
    }

    if (wf.completed_at) {
      meta.push(['Completed', ctx.formatTime(wf.completed_at) +
        ' (' + ctx.formatExactTime(wf.completed_at) + ')']);
    }

    if (meta.length) {
      var strip = '<div class="row-detail-meta">';
      for (var i = 0; i < meta.length; i++) {
        if (i > 0) strip += '<span class="row-detail-meta-sep">&middot;</span>';
        strip += '<span><span class="row-detail-meta-label">' + meta[i][0] +
          '</span> ' + meta[i][1] + '</span>';
      }
      strip += '</div>';
      parts.push(strip);
    }

    // Input — skip if empty object / empty string / just whitespace.
    var inputStr = wf.input != null ? String(wf.input).trim() : '';
    if (inputStr && inputStr !== '{}' && inputStr !== '[]' && inputStr !== 'null') {
      parts.push('<h4 class="detail-subhead">Input</h4>' +
        '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.input)) + '</div>');
    }

    if (wf.result) {
      var resultStr = String(wf.result).trim();
      if (resultStr && resultStr !== 'null') {
        parts.push('<h4 class="detail-subhead">Result</h4>' +
          '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(wf.result)) + '</div>');
      }
    }

    if (wf.error) {
      parts.push('<h4 class="detail-subhead" style="color: var(--red);">Error</h4>' +
        '<div class="error-box">' + ctx.escapeHtml(wf.error) + '</div>');
    }

    // Search attributes — skip the auto-stamped engine version line
    // since that's infra detail, not user-meaningful. Show the rest.
    if (wf.search_attributes) {
      try {
        var attrs = typeof wf.search_attributes === 'string'
          ? JSON.parse(wf.search_attributes)
          : wf.search_attributes;
        var keys = Object.keys(attrs || {}).filter(function (k) {
          return k !== 'assay_engine_version';
        });
        if (keys.length) {
          var filtered = {};
          for (var k = 0; k < keys.length; k++) filtered[keys[k]] = attrs[keys[k]];
          parts.push('<h4 class="detail-subhead">Search attributes</h4>' +
            '<div class="json-viewer">' + ctx.escapeHtml(ctx.formatJson(JSON.stringify(filtered))) + '</div>');
        }
      } catch (_) { /* malformed — skip */ }
    }

    if (parts.length === 0) {
      return '<p class="detail-muted">No run-level context recorded yet.</p>';
    }
    return parts.join('');
  }

  // Seconds → "1h 23m 45s" / "3m 12s" / "45s" / "120ms" — same
  // granularity style formatTime uses for ages, but duration-flavour.
  function formatDuration(secs) {
    if (secs < 1) return Math.round(secs * 1000) + 'ms';
    if (secs < 60) return Math.round(secs) + 's';
    if (secs < 3600) {
      var m = Math.floor(secs / 60);
      var s = Math.round(secs % 60);
      return m + 'm' + (s ? ' ' + s + 's' : '');
    }
    var h = Math.floor(secs / 3600);
    var rem = secs - h * 3600;
    var mm = Math.floor(rem / 60);
    return h + 'h' + (mm ? ' ' + mm + 'm' : '');
  }

  // ── Pipeline tab — shape extraction ─────────────────────────────
  //
  // The state snapshot is the merged result of every `register_query`
  // handler the workflow set up, keyed by query name. The Pipeline tab
  // convention is one specific query: `pipeline_state`, returning at
  // minimum a `steps[]` array. Anything else is treated as "no
  // pipeline" and the tab is hidden entirely. See
  // docs/modules/workflow.md for the full schema.
  function extractPipelineState(stateResp) {
    if (!stateResp || stateResp.state == null) return null;
    var snap = stateResp.state;
    var ps = (snap && typeof snap === 'object') ? snap.pipeline_state : null;
    if (!ps || !Array.isArray(ps.steps) || ps.steps.length === 0) return null;
    return {
      status: ps.status || null,
      current_step: ps.current_step || null,
      steps: ps.steps,
      log: Array.isArray(ps.log) ? ps.log : [],
      raw: ps,
    };
  }

  // When a workflow ends terminally without the handler running cleanup
  // (Kill / SIGKILL, or Cancel on a handler that ignored the raise), the
  // snapshot freezes mid-flight — the running step stays "running"
  // forever because no replay happens. Overlay the engine's terminal
  // status onto the display so operators don't see stale "running"
  // circles on a dead workflow. Raw snapshot stays untouched (audit
  // correctness); this is purely visual.
  function applyTerminalOverlay(pipeline, workflowStatus) {
    if (!pipeline || !Array.isArray(pipeline.steps)) return;
    var s = (workflowStatus || '').toUpperCase();
    if (s !== 'FAILED' && s !== 'CANCELLED' && s !== 'TERMINATED') return;
    var interruptedStatus =
      s === 'FAILED' ? 'failed' :
      s === 'TERMINATED' ? 'failed' : 'cancelled';
    var passedInterrupt = false;
    for (var i = 0; i < pipeline.steps.length; i++) {
      var step = pipeline.steps[i];
      var cur = (step.status || '').toLowerCase();
      if (passedInterrupt && cur === 'waiting') {
        step.status = 'skipped';
      } else if (cur === 'running') {
        step.status = interruptedStatus;
        passedInterrupt = true;
      } else if (cur === 'waiting' && !passedInterrupt) {
        // No running step found yet but workflow is dead — this step
        // never ran either. Mark skipped.
        step.status = 'skipped';
      }
    }
  }

  function renderPipelineHtml(wf, pipeline) {
    var idAttr = ctx.escapeHtml(wf.id);
    var terminal = ctx.isTerminal((wf.status || 'PENDING').toUpperCase());
    var html =
      '<div class="pipeline-tab"' +
        ' data-workflow-id="' + idAttr + '"' +
        ' data-terminal="' + (terminal ? 'true' : 'false') + '"' +
        ' data-scroll-lock="false"' +
      '>';

    html += '<div class="pipeline-strip">';
    for (var i = 0; i < pipeline.steps.length; i++) {
      var step = pipeline.steps[i];
      var view = stepView(step.status);
      var nameAttr = ctx.escapeHtml(step.name || ('Step ' + (i + 1)));
      html += '<div class="pipeline-step ' + view.cls + '"' +
                ' data-step-idx="' + (i + 1) + '"' +
                ' data-step-name="' + nameAttr + '">' +
                '<div class="step-circle">' + view.glyph + '</div>' +
                '<div class="step-label">' + nameAttr + '</div>' +
                '<div class="step-status">' + ctx.escapeHtml(step.status || 'waiting') + '</div>';
      // `view.glyph` for the running state is HTML (the SVG spinner)
      // so it goes through innerHTML naturally — escapeHtml here would
      // double-escape. Other states are plain unicode chars where
      // innerHTML and escapeHtml give the same output.
      if (Array.isArray(step.actions) && step.actions.length) {
        html += '<div class="step-actions">';
        for (var a = 0; a < step.actions.length; a++) {
          var actName = String(step.actions[a]);
          html +=
            '<button class="pipeline-step-action"' +
              ' data-workflow-id="' + idAttr + '"' +
              ' data-step-name="' + nameAttr + '"' +
              ' data-action="' + ctx.escapeHtml(actName) + '">' +
              ctx.escapeHtml(actName) +
            '</button>';
        }
        html += '</div>';
      }
      // Countdown badge — only on running steps with `expires_at`
      // (the workflow author opts in by setting it). Live-ticked by
      // tickStepCountdowns() every second.
      if (step.expires_at && (step.status || '').toLowerCase() === 'running') {
        html += '<div class="step-expires" data-expires-at="' +
                  Number(step.expires_at) + '">' +
                  formatRemaining(step.expires_at) +
                '</div>';
      }
      html += '</div>'; // /pipeline-step

      if (i < pipeline.steps.length - 1) {
        var fill = connectorFill(step.status, pipeline.steps[i + 1].status);
        html += '<div class="pipeline-connector ' + fill + '"></div>';
      }
    }
    html += '</div>'; // /pipeline-strip

    html += '<div class="pipeline-log-header">';
    html += '<span class="pipeline-log-title">Step log</span>';
    html += '<span class="pipeline-live ' + (terminal ? 'off' : 'on') + '">' +
              (terminal ? 'final' : '\u25CF live') +
            '</span>';
    html += '<span class="pipeline-log-spacer"></span>';
    html += '<button class="pipeline-scroll-lock" type="button">\u25BE Auto-scroll</button>';
    html += '</div>';

    html += '<div class="pipeline-log">';
    html += renderLogLines(pipeline.log);
    html += '</div>';

    html += '</div>'; // /pipeline-tab
    return html;
  }

  function renderLogLines(logArr) {
    if (!logArr || !logArr.length) {
      return '<div class="pipeline-log-empty">No log entries yet.</div>';
    }
    var out = '';
    for (var i = 0; i < logArr.length; i++) {
      out += renderLogLine(logArr[i]);
    }
    return out;
  }

  function renderLogLine(entry) {
    var time = ctx.escapeHtml(String(entry.time || ''));
    var msg = ctx.escapeHtml(String(entry.msg || ''));
    var stepIdx = entry.step != null ? entry.step : entry.stage;
    var stepAttr = stepIdx != null ? (' data-step-idx="' + Number(stepIdx) + '"') : '';
    return '<div class="pipeline-log-line"' + stepAttr + '>' +
             '<span class="log-time">' + time + '</span>' +
             '<span class="log-msg">' + msg + '</span>' +
           '</div>';
  }

  function applyLogFilter(pipelineRoot) {
    var filter = pipelineRoot.dataset.filterStep || null;
    var lines = pipelineRoot.querySelectorAll('.pipeline-log-line');
    for (var i = 0; i < lines.length; i++) {
      var match = filter == null || lines[i].dataset.stepIdx === filter;
      lines[i].classList.toggle('hidden', !match);
    }
  }

  // ── Pipeline tab — live polling ─────────────────────────────────
  //
  // While the Pipeline tab is open and the workflow hasn't reached a
  // terminal status, poll /workflows/{id}/state/pipeline_state every
  // 1s and diff-apply changes onto the existing DOM. We never re-render
  // the whole tab — that would reset the user's scroll position in the
  // log and kill any in-progress animations on the running circle.
  function startPipelinePoller(workflowId, scopeEl) {
    var POLL_MS = 1000;

    // Find the .pipeline-tab element under the active panel scope.
    function findRoot() {
      // scopeEl may be the panel itself or a tab container; querySelector
      // resolves to the first match either way.
      return (scopeEl || document).querySelector('.pipeline-tab[data-workflow-id="' + cssEscape(workflowId) + '"]')
        || document.querySelector('.pipeline-tab[data-workflow-id="' + cssEscape(workflowId) + '"]');
    }

    var handle = { id: workflowId, interval: null, lastEventSeq: -1, lastLogLen: 0 };
    activePoller = handle;

    async function tick() {
      var root = findRoot();
      if (!root || root.dataset.terminal === 'true') {
        stopPoller();
        return;
      }
      try {
        var resp = await ctx.apiFetch(
          '/workflows/' + encodeURIComponent(workflowId) + '/state/pipeline_state'
        );
        if (!resp || resp.value == null) return;
        if (typeof resp.event_seq === 'number' && resp.event_seq === handle.lastEventSeq) {
          // Snapshot hasn't advanced since last poll — skip work.
          return;
        }
        handle.lastEventSeq = resp.event_seq != null ? resp.event_seq : handle.lastEventSeq;
        applyPipelineUpdate(root, resp.value, handle);
      } catch (_) {
        // Transient errors during a 1Hz poll are common (engine pod
        // restart, network blip). Swallow them silently and let the
        // next tick try again — the worker resilience pattern from
        // v0.11.14 applies here too.
      }
    }

    handle.interval = setInterval(tick, POLL_MS);
    // Capture the initial log length so the first diff doesn't append
    // entries that are already on screen from the initial render.
    var rootNow = findRoot();
    if (rootNow) {
      handle.lastLogLen = rootNow.querySelectorAll('.pipeline-log-line').length;
    }
    // Run immediately so operators don't wait a full second for the
    // first refresh after the tab opens.
    tick();
  }

  function stopPoller() {
    if (activePoller && activePoller.interval) {
      clearInterval(activePoller.interval);
    }
    activePoller = null;
  }

  function applyPipelineUpdate(root, value, handle) {
    if (!value || !Array.isArray(value.steps)) return;

    // Update step circles + connectors in place.
    var stepEls = root.querySelectorAll('.pipeline-step');
    var connectorEls = root.querySelectorAll('.pipeline-connector');
    for (var i = 0; i < stepEls.length; i++) {
      var s = value.steps[i];
      if (!s) continue;
      var view = stepView(s.status);
      var el = stepEls[i];
      // Reset state classes and reapply.
      el.classList.remove('waiting', 'running', 'done', 'failed', 'cancelled', 'skipped');
      el.classList.add(view.cls);
      var glyphEl = el.querySelector('.step-circle');
      if (glyphEl) {
        // The running glyph is SVG markup (innerHTML); other statuses
        // are plain unicode chars (textContent works for both, but
        // setting innerHTML makes the SVG path render as a node).
        if (view.isHtml) {
          if (!glyphEl.querySelector('.step-spinner')) {
            glyphEl.innerHTML = view.glyph;
          }
        } else if (glyphEl.textContent !== view.glyph) {
          glyphEl.textContent = view.glyph;
        }
      }
      var statusEl = el.querySelector('.step-status');
      if (statusEl) statusEl.textContent = s.status || 'waiting';
      reconcileStepActions(el, s, root.dataset.workflowId);
      reconcileStepExpires(el, s);
    }
    for (var c = 0; c < connectorEls.length; c++) {
      var prev = value.steps[c] && value.steps[c].status;
      var next = value.steps[c + 1] && value.steps[c + 1].status;
      var fill = connectorFill(prev, next);
      connectorEls[c].classList.remove('full', 'partial', 'muted');
      connectorEls[c].classList.add(fill);
    }

    // Auto-advance focus to the running step (or `current_step` if the
    // workflow set it explicitly). Skipped when the user manually
    // selected a step — their choice sticks until they click again to
    // release. Keeps the log filter aligned with whatever's actually
    // happening right now.
    if (root.dataset.userSelected !== 'true') {
      autoAdvanceFocus(root, value);
    }

    // Diff-append log lines.
    var logArr = Array.isArray(value.log) ? value.log : [];
    if (logArr.length > handle.lastLogLen) {
      var logEl = root.querySelector('.pipeline-log');
      if (logEl) {
        var emptyEl = logEl.querySelector('.pipeline-log-empty');
        if (emptyEl) emptyEl.remove();
        var added = '';
        for (var l = handle.lastLogLen; l < logArr.length; l++) {
          added += renderLogLine(logArr[l]);
        }
        logEl.insertAdjacentHTML('beforeend', added);
        handle.lastLogLen = logArr.length;
        applyLogFilter(root);
        if (root.dataset.scrollLock !== 'true') {
          logEl.scrollTop = logEl.scrollHeight;
        }
      }
    }
  }

  // Pick the step the operator most likely cares about and select it,
  // unless they've already chosen one manually (handled by the caller).
  // Priority: explicit `current_step` from the snapshot → first
  // `running` step → first `failed` step → no selection.
  function autoAdvanceFocus(root, value) {
    var idx = null;
    if (typeof value.current_step === 'number' && value.current_step > 0) {
      idx = value.current_step;
    } else if (Array.isArray(value.steps)) {
      for (var i = 0; i < value.steps.length; i++) {
        if ((value.steps[i].status || '').toLowerCase() === 'running') {
          idx = i + 1; break;
        }
      }
      if (idx == null) {
        for (var j = 0; j < value.steps.length; j++) {
          if ((value.steps[j].status || '').toLowerCase() === 'failed') {
            idx = j + 1; break;
          }
        }
      }
    }
    var stepEls = root.querySelectorAll('.pipeline-step');
    var changed = false;
    for (var k = 0; k < stepEls.length; k++) {
      var want = String(k + 1) === String(idx);
      var has = stepEls[k].classList.contains('selected');
      if (want && !has) { stepEls[k].classList.add('selected'); changed = true; }
      else if (!want && has) { stepEls[k].classList.remove('selected'); changed = true; }
    }
    if (idx != null) {
      root.dataset.filterStep = String(idx);
    } else {
      delete root.dataset.filterStep;
    }
    if (changed) applyLogFilter(root);
  }

  // Format remaining time until a unix-epoch deadline as "Xm Ys" or
  // "Xs" (or "—" if already past). Used by the step countdown badge
  // so operators can see exactly how long a wait_for_signal has
  // before it times out.
  function formatRemaining(epochSecs) {
    var now = Math.floor(Date.now() / 1000);
    var rem = Math.max(0, Number(epochSecs) - now);
    if (rem <= 0) return 'expired';
    if (rem < 60) return rem + 's';
    var m = Math.floor(rem / 60);
    var s = rem % 60;
    if (m < 60) return m + 'm ' + (s ? s + 's' : '');
    var h = Math.floor(m / 60);
    var mm = m % 60;
    return h + 'h ' + mm + 'm';
  }

  // Tick all visible step countdowns once per second. Started lazily
  // by startCountdownTicker(); stopped when no .step-expires nodes
  // remain (e.g. after the step transitions out of running and the
  // poller's diff-update reconciles them away).
  var countdownInterval = null;
  function startCountdownTicker() {
    if (countdownInterval) return;
    countdownInterval = setInterval(function () {
      var nodes = document.querySelectorAll('.step-expires[data-expires-at]');
      if (!nodes.length) {
        clearInterval(countdownInterval);
        countdownInterval = null;
        return;
      }
      var nowSecs = Math.floor(Date.now() / 1000);
      for (var i = 0; i < nodes.length; i++) {
        var ea = Number(nodes[i].dataset.expiresAt);
        nodes[i].textContent = formatRemaining(ea);
        // Visual urgency at the last 60s.
        if (ea - nowSecs <= 60) {
          nodes[i].classList.add('step-expires-soon');
        } else {
          nodes[i].classList.remove('step-expires-soon');
        }
      }
    }, 1000);
  }

  // Add/remove the live countdown badge in response to a fresh
  // snapshot. Workflows clear `expires_at` when the wait completes
  // (signal arrived, timer fired, step ended) and the badge needs
  // to disappear in lockstep without a full re-render.
  function reconcileStepExpires(stepEl, step) {
    var existing = stepEl.querySelector('.step-expires');
    var hasDeadline = step.expires_at &&
      (step.status || '').toLowerCase() === 'running';
    if (!hasDeadline) {
      if (existing) existing.remove();
      return;
    }
    var html = formatRemaining(step.expires_at);
    if (existing) {
      if (Number(existing.dataset.expiresAt) !== Number(step.expires_at)) {
        existing.dataset.expiresAt = String(step.expires_at);
      }
      if (existing.textContent !== html) existing.textContent = html;
    } else {
      stepEl.insertAdjacentHTML('beforeend',
        '<div class="step-expires" data-expires-at="' + Number(step.expires_at) + '">' +
          html +
        '</div>');
      startCountdownTicker();
    }
  }

  // Add/remove the per-step action buttons in response to a fresh
  // snapshot. Workflows commonly clear `actions` once a step transitions
  // (e.g. dropping the Approve/Reject buttons after the decision lands)
  // and we want that to happen without a full tab re-render so the
  // log scroll position and animations stay intact.
  function reconcileStepActions(stepEl, step, workflowId) {
    var existing = stepEl.querySelector('.step-actions');
    var actions = Array.isArray(step.actions) ? step.actions : [];
    if (!actions.length) {
      if (existing) existing.remove();
      return;
    }
    var stepName = stepEl.dataset.stepName || step.name || '';
    var html = '';
    for (var a = 0; a < actions.length; a++) {
      var actName = String(actions[a]);
      var safeAct = ctx.escapeHtml(actName);
      var safeName = ctx.escapeHtml(stepName);
      var safeId = ctx.escapeHtml(workflowId || '');
      html +=
        '<button class="pipeline-step-action"' +
          ' data-workflow-id="' + safeId + '"' +
          ' data-step-name="' + safeName + '"' +
          ' data-action="' + safeAct + '">' +
          safeAct +
        '</button>';
    }
    if (existing) {
      if (existing.innerHTML !== html) existing.innerHTML = html;
    } else {
      stepEl.insertAdjacentHTML('beforeend',
        '<div class="step-actions">' + html + '</div>');
    }
  }

  async function handleStepAction(workflowId, stepName, action) {
    if (!workflowId || !stepName || !action) return;
    var who = (typeof window !== 'undefined' && window.localStorage)
      ? window.localStorage.getItem('assay.user') || ''
      : '';
    try {
      await ctx.apiFetch(
        '/workflows/' + encodeURIComponent(workflowId) + '/signal/step_action',
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            payload: { step: stepName, action: action, user: who || null },
          }),
        }
      );
      ctx.toast(action + ' \u2192 ' + stepName, 'success');
      // Snapshot will catch up via the 1Hz poller; no full re-render
      // needed.
    } catch (err) {
      ctx.toast('Action failed: ' + (err && err.message), 'error');
    }
  }

  // CSS.escape isn't universal in older browsers and our workflow ids
  // can contain `.` (e.g. `promo-v0.1.0-rc24`) which would break a raw
  // attribute selector. Minimal escape for the chars we actually care
  // about in workflow ids.
  function cssEscape(s) {
    return String(s).replace(/(["\\])/g, '\\$1');
  }

  return { showDetail: showDetail, closeDetail: closeDetail };
})();
