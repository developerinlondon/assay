/* Assay Workflow Dashboard — workflow action handlers (v0.12.0)
 *
 * Signal / Cancel / Terminate / Continue-as-new — one entry point per
 * action. Each opens an AssayModal for input/confirmation, posts to
 * the engine, surfaces a toast on success/failure, and refreshes the
 * affected views. Callers (workflow row icons, detail-panel keyboard
 * shortcuts, future programmatic flows) just hand over the workflow
 * id; the actions own the UI and the API plumbing.
 */

var AssayActions = (function () {
  'use strict';

  var ctx = null;
  function init(c) { ctx = c; }

  function refreshList() {
    if (ctx && typeof ctx.refreshCurrentView === 'function') ctx.refreshCurrentView();
  }

  function reopenDetail(id) {
    // Re-render the detail panel against fresh state. AssayDetail.showDetail
    // re-fetches; cheaper than threading a partial-update path through
    // every action.
    if (typeof AssayDetail !== 'undefined' && AssayDetail.showDetail && id) {
      var panel = document.getElementById('detail-panel');
      if (panel && panel.classList.contains('open')) AssayDetail.showDetail(id, ctx);
    }
  }

  function signal(id) {
    AssayModal.form({
      title: 'Send signal to ' + id,
      submitLabel: 'Send',
      fields: [
        {
          name: 'name',
          label: 'Signal name',
          type: 'text',
          placeholder: 'e.g. approve, decision, step_action',
          required: true,
          hint: 'Power-user / ad-hoc tool. For most interactions (approve, ' +
                'reject, retry, skip) use the per-step buttons on the ' +
                'Pipeline tab instead — those route through the ' +
                'step_action convention automatically. Use this form ' +
                'only when you need to send a custom signal the workflow ' +
                'registered via ctx:wait_for_signal("<name>").',
        },
        {
          name: 'payload',
          label: 'Payload (JSON, optional)',
          type: 'json',
          placeholder: '{ "user": "alice", "action": "approve" }',
          hint: 'Parsed and delivered to the workflow under the "payload" ' +
                'field. Leave empty for a payload-less signal.',
        },
      ],
      onSubmit: async function (values) {
        var name = String(values.name || '').trim();
        var payload = values.payload != null ? values.payload : null;
        try {
          await ctx.apiFetch(
            '/workflows/' + encodeURIComponent(id) + '/signal/' + encodeURIComponent(name),
            {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ payload: payload }),
            }
          );
          ctx.toast("Signal '" + name + "' sent", 'success');
          // Deliberately no refreshList here — a signal arriving
          // doesn't change the row's status. The Pipeline tab's own
          // poller will pick up the resulting state change. Refreshing
          // would collapse any inline row expansion the operator has
          // open on this workflow.
        } catch (err) {
          ctx.toast('Signal failed: ' + (err && err.message), 'error');
        }
      },
    });
  }

  function cancel(id) {
    AssayModal.form({
      title: 'Cancel ' + id + '?',
      submitLabel: 'Cancel workflow',
      description:
        'Graceful stop (SIGTERM-style). The workflow handler catches the ' +
        'cancellation, runs its cleanup code (release locks, roll back ' +
        'partial work, log the cancellation), then exits with status ' +
        'CANCELLED. Any currently-running activity completes first. Use ' +
        'this unless the workflow is stuck.',
      fields: [
        {
          name: 'reason',
          label: 'Reason (optional)',
          type: 'textarea',
          placeholder: 'Why this run is being cancelled. Recorded in the workflow\'s WorkflowCancelRequested event for audit.',
        },
      ],
      onSubmit: async function (values) {
        var body = values.reason && String(values.reason).trim()
          ? { reason: String(values.reason) } : {};
        try {
          await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/cancel', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
          });
          ctx.toast('Cancel requested', 'success');
          reopenDetail(id);
          refreshList();
        } catch (err) {
          ctx.toast('Cancel failed: ' + (err && err.message), 'error');
        }
      },
    });
  }

  function terminate(id) {
    AssayModal.form({
      title: 'Kill ' + id + '?',
      submitLabel: 'Kill workflow',
      danger: true,
      description:
        'Force-kill (SIGKILL-style). Immediately flips the workflow to ' +
        'FAILED without invoking its handler — NO cleanup code runs, ' +
        'in-flight activities are orphaned. Use Cancel first unless the ' +
        'workflow is stuck or refusing to respond.',
      fields: [
        {
          name: 'reason',
          label: 'Reason (optional)',
          type: 'textarea',
          placeholder: 'Why this run is being killed. Recorded in the workflow\'s event history for audit.',
        },
      ],
      onSubmit: async function (values) {
        var body = values.reason && String(values.reason).trim() ? { reason: String(values.reason) } : {};
        try {
          await ctx.apiFetch('/workflows/' + encodeURIComponent(id) + '/terminate', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify(body),
          });
          ctx.toast('Killed', 'success');
          reopenDetail(id);
          refreshList();
        } catch (err) {
          ctx.toast('Kill failed: ' + (err && err.message), 'error');
        }
      },
    });
  }

  function continueAsNew(id) {
    // Smart default for the new id: strip any existing
    // `-continued-<digits>` suffix from the source so sequential
    // continues don't stack ( demo-1 → demo-1-continued-1 → demo-1-
    // continued-2 rather than demo-1-continued-1-continued-2 ). Users
    // can still edit freely before submit.
    var base = String(id || '').replace(/-continued-\d+$/, '');
    var suggestedId = base + '-continued-' + Math.floor(Date.now() / 1000);

    AssayModal.form({
      title: 'Start a new run — ' + id,
      submitLabel: 'Start new run',
      description:
        'Starts a NEW run with the same type + queue. Durable-workflow ' +
        'histories are immutable — the existing run stays in its terminal ' +
        'state for audit; the new run has its own fresh history. Pick an ' +
        'id you\'ll recognise later; the default strips any previous ' +
        '-continued- suffix so chained continues don\'t pile up.',
      fields: [
        {
          name: 'workflow_id',
          label: 'New workflow ID',
          type: 'text',
          value: suggestedId,
          required: true,
        },
        {
          name: 'input',
          label: 'New input (JSON, optional)',
          type: 'json',
          placeholder: '{ "version": "v0.2.0", "target": "qa" }',
        },
      ],
      onSubmit: async function (values) {
        var body = {};
        if (values.input != null) body.input = values.input;
        if (values.workflow_id && String(values.workflow_id).trim()) {
          body.workflow_id = String(values.workflow_id).trim();
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
          var newId = newRun && newRun.workflow_id;
          ctx.toast('New run started: ' + (newId || 'unknown'), 'success');
          // If the user had an inline expansion open on the workflow
          // they continued-as-new FROM, transfer it to the new run —
          // the mental model is "I was tracking this pipeline, now
          // I want to track its continuation". Implemented as a
          // best-effort DOM handoff: we mark the new id as the one
          // to re-expand; the upcoming list refresh (triggered by
          // SSE workflow_started) picks it up via restoreExpandedRow.
          if (newId && typeof window !== 'undefined' && window.AssayWorkflows
              && window.AssayWorkflows.setExpandedId) {
            window.AssayWorkflows.setExpandedId(newId);
          }
        } catch (err) {
          ctx.toast('Continue-as-new failed: ' + (err && err.message), 'error');
        }
      },
    });
  }

  return {
    init: init,
    signal: signal,
    cancel: cancel,
    terminate: terminate,
    continueAsNew: continueAsNew,
  };
})();
