/* Assay Engine Console — shared API client.
 *
 * Thin wrapper around fetch() that bakes the admin bearer token into
 * every request and surfaces non-2xx responses as Errors with the
 * server's `error_description` (when present) so the UI can toast a
 * meaningful message.
 */

(function () {
  'use strict';

  function getToken() {
    if (window.AssayEngineApp && window.AssayEngineApp.getToken) {
      return window.AssayEngineApp.getToken();
    }
    return localStorage.getItem('assay-admin-token') || '';
  }

  async function call(method, path, body, opts) {
    const headers = { 'accept': 'application/json' };
    const token = getToken();
    if (token) headers['authorization'] = 'Bearer ' + token;
    if (body !== undefined && body !== null) {
      headers['content-type'] = 'application/json';
    }
    const init = {
      method: method,
      headers: headers,
    };
    if (body !== undefined && body !== null) init.body = JSON.stringify(body);
    const r = await fetch(path, init);
    if (!r.ok) {
      let msg = 'HTTP ' + r.status;
      try {
        const j = await r.json();
        if (j && j.error_description) msg = j.error_description;
        else if (j && j.error) msg = j.error;
      } catch (_) { /* non-JSON body — use status code */ }
      const err = new Error(msg);
      err.status = r.status;
      throw err;
    }
    if (opts && opts.raw) return r;
    if (r.status === 204) return null;
    return await r.json();
  }

  window.AssayEngineApi = {
    info: function () { return call('GET', '/api/v1/engine/core/info'); },
    listModules: function () { return call('GET', '/api/v1/engine/core/modules'); },
    toggleModule: function (name, enabled) {
      return call('POST', '/api/v1/engine/core/modules/' + encodeURIComponent(name) + '/toggle',
        { enabled: enabled });
    },
    listInstances: function () { return call('GET', '/api/v1/engine/core/instances'); },
    listAudit: function (q) {
      const p = new URLSearchParams();
      if (q && q.limit !== undefined) p.set('limit', String(q.limit));
      if (q && q.offset !== undefined) p.set('offset', String(q.offset));
      if (q && q.actor) p.set('actor', q.actor);
      if (q && q.action) p.set('action', q.action);
      if (q && q.since !== undefined) p.set('since', String(q.since));
      if (q && q.until !== undefined) p.set('until', String(q.until));
      const qs = p.toString();
      return call('GET', '/api/v1/engine/core/audit' + (qs ? ('?' + qs) : ''));
    },
    getConfig: function () { return call('GET', '/api/v1/engine/core/config'); },
  };
})();
