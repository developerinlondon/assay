// assay workflow dashboard
// Real-time updates via SSE, API calls via fetch

const API = '/api/v1';
let currentView = 'workflows';
let eventSource = null;

// ── Navigation ──────────────────────────────────────────────

document.querySelectorAll('.nav-link').forEach(link => {
    link.addEventListener('click', e => {
        e.preventDefault();
        const tab = link.dataset.tab;
        setView(tab);
    });
});

function setView(view) {
    currentView = view;
    document.querySelectorAll('.nav-link').forEach(l => l.classList.remove('active'));
    document.querySelector(`[data-tab="${view}"]`)?.classList.add('active');
    closeDetail();

    const content = document.getElementById('content');
    if (view === 'workflows') {
        content.innerHTML = workflowsTemplate();
        loadWorkflows();
    } else if (view === 'schedules') {
        content.innerHTML = schedulesTemplate();
        loadSchedules();
    } else if (view === 'workers') {
        content.innerHTML = workersTemplate();
        loadWorkers();
    }
}

// ── SSE Real-time Updates ───────────────────────────────────

function connectSSE() {
    if (eventSource) eventSource.close();
    eventSource = new EventSource(`${API}/events/stream`);

    eventSource.addEventListener('WorkflowStarted', () => {
        if (currentView === 'workflows') loadWorkflows();
    });
    eventSource.addEventListener('WorkflowCompleted', () => {
        if (currentView === 'workflows') loadWorkflows();
    });
    eventSource.addEventListener('WorkflowFailed', () => {
        if (currentView === 'workflows') loadWorkflows();
    });
    eventSource.addEventListener('WorkflowCancelled', () => {
        if (currentView === 'workflows') loadWorkflows();
    });
    eventSource.addEventListener('ActivityCompleted', () => {
        if (currentView === 'workflows') loadWorkflows();
    });

    eventSource.onerror = () => {
        setTimeout(connectSSE, 3000);
    };
}

connectSSE();

// ── Templates ───────────────────────────────────────────────

function workflowsTemplate() {
    return `
        <section id="workflows-view">
            <div class="toolbar">
                <h2><span class="live-dot"></span>Workflows</h2>
                <div class="filters">
                    <select id="status-filter" onchange="loadWorkflows()">
                        <option value="">All statuses</option>
                        <option value="PENDING">Pending</option>
                        <option value="RUNNING">Running</option>
                        <option value="WAITING">Waiting</option>
                        <option value="COMPLETED">Completed</option>
                        <option value="FAILED">Failed</option>
                        <option value="CANCELLED">Cancelled</option>
                    </select>
                    <button onclick="loadWorkflows()" class="btn btn-sm">Refresh</button>
                </div>
            </div>
            <table class="data-table">
                <thead><tr>
                    <th>ID</th><th>Type</th><th>Status</th><th>Queue</th><th>Created</th><th>Actions</th>
                </tr></thead>
                <tbody id="workflows-body">
                    <tr><td colspan="6" class="empty">Loading...</td></tr>
                </tbody>
            </table>
        </section>
        <div id="detail-panel" class="detail-panel hidden">
            <div class="detail-header">
                <h2 id="detail-title">Workflow Detail</h2>
                <button onclick="closeDetail()" class="btn btn-sm">Close</button>
            </div>
            <div id="detail-content"></div>
        </div>`;
}

function schedulesTemplate() {
    return `
        <section>
            <div class="toolbar">
                <h2>Schedules</h2>
                <button onclick="loadSchedules()" class="btn btn-sm">Refresh</button>
            </div>
            <table class="data-table">
                <thead><tr>
                    <th>Name</th><th>Workflow Type</th><th>Cron</th><th>Queue</th><th>Last Run</th><th>Actions</th>
                </tr></thead>
                <tbody id="schedules-body">
                    <tr><td colspan="6" class="empty">Loading...</td></tr>
                </tbody>
            </table>
        </section>`;
}

function workersTemplate() {
    return `
        <section>
            <div class="toolbar">
                <h2>Workers</h2>
                <button onclick="loadWorkers()" class="btn btn-sm">Refresh</button>
            </div>
            <table class="data-table">
                <thead><tr>
                    <th>ID</th><th>Identity</th><th>Queue</th><th>Active Tasks</th><th>Last Heartbeat</th>
                </tr></thead>
                <tbody id="workers-body">
                    <tr><td colspan="5" class="empty">Loading...</td></tr>
                </tbody>
            </table>
        </section>`;
}

// ── Data Loading ────────────────────────────────────────────

async function loadWorkflows() {
    const filter = document.getElementById('status-filter')?.value || '';
    const params = filter ? `?status=${filter}` : '';
    try {
        const resp = await fetch(`${API}/workflows${params}`);
        const workflows = await resp.json();
        const body = document.getElementById('workflows-body');
        if (!body) return;

        if (workflows.length === 0) {
            body.innerHTML = '<tr><td colspan="6" class="empty">No workflows found</td></tr>';
            return;
        }

        body.innerHTML = workflows.map(wf => `
            <tr>
                <td><a class="id-link" onclick="showWorkflow('${wf.id}')">${truncate(wf.id, 32)}</a></td>
                <td>${wf.workflow_type}</td>
                <td><span class="status status-${wf.status}">${wf.status}</span></td>
                <td>${wf.task_queue}</td>
                <td>${formatTime(wf.created_at)}</td>
                <td>
                    ${!isTerminal(wf.status) ? `
                        <button onclick="signalWorkflow('${wf.id}')" class="btn btn-sm">Signal</button>
                        <button onclick="cancelWorkflow('${wf.id}')" class="btn btn-sm btn-danger">Cancel</button>
                    ` : ''}
                </td>
            </tr>
        `).join('');
    } catch (e) {
        console.error('Failed to load workflows:', e);
    }
}

async function loadSchedules() {
    try {
        const resp = await fetch(`${API}/schedules`);
        const schedules = await resp.json();
        const body = document.getElementById('schedules-body');
        if (!body) return;

        if (schedules.length === 0) {
            body.innerHTML = '<tr><td colspan="6" class="empty">No schedules configured</td></tr>';
            return;
        }

        body.innerHTML = schedules.map(s => `
            <tr>
                <td><strong>${s.name}</strong></td>
                <td>${s.workflow_type}</td>
                <td><code>${s.cron_expr}</code></td>
                <td>${s.task_queue}</td>
                <td>${s.last_run_at ? formatTime(s.last_run_at) : 'Never'}</td>
                <td>
                    <button onclick="deleteSchedule('${s.name}')" class="btn btn-sm btn-danger">Delete</button>
                </td>
            </tr>
        `).join('');
    } catch (e) {
        console.error('Failed to load schedules:', e);
    }
}

async function loadWorkers() {
    try {
        const resp = await fetch(`${API}/workers`);
        const workers = await resp.json();
        const body = document.getElementById('workers-body');
        if (!body) return;

        if (workers.length === 0) {
            body.innerHTML = '<tr><td colspan="5" class="empty">No workers connected</td></tr>';
            return;
        }

        body.innerHTML = workers.map(w => `
            <tr>
                <td><code>${truncate(w.id, 20)}</code></td>
                <td>${w.identity}</td>
                <td>${w.task_queue}</td>
                <td>${w.active_tasks}</td>
                <td>${formatTime(w.last_heartbeat)}</td>
            </tr>
        `).join('');
    } catch (e) {
        console.error('Failed to load workers:', e);
    }
}

// ── Workflow Detail ─────────────────────────────────────────

async function showWorkflow(id) {
    try {
        const [wfResp, evResp] = await Promise.all([
            fetch(`${API}/workflows/${id}`),
            fetch(`${API}/workflows/${id}/events`),
        ]);
        const wf = await wfResp.json();
        const events = await evResp.json();

        document.getElementById('detail-title').textContent = wf.id;
        document.getElementById('detail-content').innerHTML = `
            <div class="meta-grid">
                <span class="meta-label">Status</span>
                <span><span class="status status-${wf.status}">${wf.status}</span></span>
                <span class="meta-label">Type</span>
                <span class="meta-value">${wf.workflow_type}</span>
                <span class="meta-label">Run ID</span>
                <span class="meta-value">${wf.run_id}</span>
                <span class="meta-label">Queue</span>
                <span class="meta-value">${wf.task_queue}</span>
                <span class="meta-label">Created</span>
                <span class="meta-value">${formatTime(wf.created_at)}</span>
                ${wf.completed_at ? `
                    <span class="meta-label">Completed</span>
                    <span class="meta-value">${formatTime(wf.completed_at)}</span>
                ` : ''}
                ${wf.input ? `
                    <span class="meta-label">Input</span>
                    <span class="meta-value">${formatJson(wf.input)}</span>
                ` : ''}
                ${wf.result ? `
                    <span class="meta-label">Result</span>
                    <span class="meta-value">${formatJson(wf.result)}</span>
                ` : ''}
                ${wf.error ? `
                    <span class="meta-label">Error</span>
                    <span class="meta-value" style="color:var(--red)">${wf.error}</span>
                ` : ''}
            </div>
            <div class="timeline">
                <h3>Event History (${events.length})</h3>
                ${events.map(ev => `
                    <div class="event-item">
                        <span class="event-seq">#${ev.seq}</span>
                        <span class="event-type">${ev.event_type}</span>
                        <span class="event-time">${formatTime(ev.timestamp)}</span>
                    </div>
                `).join('')}
                ${events.length === 0 ? '<div class="empty">No events</div>' : ''}
            </div>
            ${!isTerminal(wf.status) ? `
                <div style="padding: 0 20px 20px; display: flex; gap: 8px;">
                    <button onclick="signalWorkflow('${wf.id}')" class="btn">Send Signal</button>
                    <button onclick="cancelWorkflow('${wf.id}')" class="btn btn-danger">Cancel</button>
                </div>
            ` : ''}
        `;

        document.getElementById('detail-panel').classList.remove('hidden');
    } catch (e) {
        console.error('Failed to load workflow detail:', e);
    }
}

function closeDetail() {
    document.getElementById('detail-panel')?.classList.add('hidden');
}

// ── Actions ─────────────────────────────────────────────────

async function cancelWorkflow(id) {
    if (!confirm(`Cancel workflow ${id}?`)) return;
    await fetch(`${API}/workflows/${id}/cancel`, { method: 'POST' });
    loadWorkflows();
    closeDetail();
}

async function signalWorkflow(id) {
    const name = prompt('Signal name:');
    if (!name) return;
    const payload = prompt('Payload (JSON, optional):');
    const body = payload ? { payload: JSON.parse(payload) } : {};
    await fetch(`${API}/workflows/${id}/signal/${name}`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
    });
    loadWorkflows();
}

async function deleteSchedule(name) {
    if (!confirm(`Delete schedule ${name}?`)) return;
    await fetch(`${API}/schedules/${name}`, { method: 'DELETE' });
    loadSchedules();
}

// ── Helpers ─────────────────────────────────────────────────

function formatTime(ts) {
    if (!ts) return '';
    const d = new Date(ts * 1000);
    return d.toLocaleString();
}

function truncate(s, n) {
    return s && s.length > n ? s.slice(0, n) + '...' : s;
}

function isTerminal(status) {
    return ['COMPLETED', 'FAILED', 'CANCELLED', 'TIMED_OUT'].includes(status);
}

function formatJson(s) {
    try {
        const obj = typeof s === 'string' ? JSON.parse(s) : s;
        return JSON.stringify(obj, null, 2);
    } catch {
        return s;
    }
}

// ── Init ────────────────────────────────────────────────────

loadWorkflows();
