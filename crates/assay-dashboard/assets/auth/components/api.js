/* Assay Auth Console — shared API client (phase 8b)
 *
 * Wraps fetch() with the admin Bearer token and JSON helpers. Every
 * call returns a Promise that resolves to the parsed JSON body or
 * throws an Error carrying the response body as its message.
 */

(function () {
  'use strict';

  function authHeaders() {
    const token = (window.AssayAuthApp && window.AssayAuthApp.getToken()) || '';
    const h = { 'content-type': 'application/json' };
    if (token) h.authorization = 'Bearer ' + token;
    return h;
  }

  async function call(method, path, body) {
    const opts = {
      method: method,
      headers: authHeaders(),
    };
    if (body !== undefined && body !== null && method !== 'GET' && method !== 'DELETE') {
      opts.body = JSON.stringify(body);
    }
    const url = '/auth' + path;
    const res = await fetch(url, opts);
    if (res.status === 204) return null;
    const text = await res.text();
    let parsed;
    try { parsed = text ? JSON.parse(text) : null; } catch (_) { parsed = text; }
    if (!res.ok) {
      const msg = (parsed && (parsed.error_description || parsed.error)) || text || res.statusText;
      const err = new Error(msg);
      err.status = res.status;
      err.body = parsed;
      throw err;
    }
    return parsed;
  }

  // Build a query string from an object, dropping nullish values.
  function qs(params) {
    if (!params) return '';
    const parts = [];
    Object.keys(params).forEach(function (k) {
      const v = params[k];
      if (v === null || v === undefined || v === '') return;
      parts.push(encodeURIComponent(k) + '=' + encodeURIComponent(v));
    });
    return parts.length ? '?' + parts.join('&') : '';
  }

  window.AssayAuthApi = {
    // generic
    get: function (path, params) { return call('GET', path + qs(params)); },
    post: function (path, body) { return call('POST', path, body); },
    put: function (path, body) { return call('PUT', path, body); },
    del: function (path, body) { return call('DELETE', path, body); },

    // Users
    listUsers: function (params) { return call('GET', '/admin/auth/users' + qs(params)); },
    createUser: function (body) { return call('POST', '/admin/auth/users', body); },
    getUser: function (id) { return call('GET', '/admin/auth/users/' + encodeURIComponent(id)); },
    updateUser: function (id, body) { return call('PUT', '/admin/auth/users/' + encodeURIComponent(id), body); },
    deleteUser: function (id) { return call('DELETE', '/admin/auth/users/' + encodeURIComponent(id)); },
    resetPassword: function (id, password) {
      return call('POST', '/admin/auth/users/' + encodeURIComponent(id) + '/password-reset', { password: password });
    },

    // Sessions
    listSessions: function (params) { return call('GET', '/admin/auth/sessions' + qs(params)); },
    revokeSession: function (id) { return call('DELETE', '/admin/auth/sessions/' + encodeURIComponent(id)); },
    revokeAllSessions: function (uid) {
      return call('DELETE', '/admin/auth/sessions/by-user/' + encodeURIComponent(uid));
    },

    // OIDC clients
    listOidcClients: function () { return call('GET', '/admin/oidc/clients'); },
    createOidcClient: function (body) { return call('POST', '/admin/oidc/clients', body); },
    getOidcClient: function (id) { return call('GET', '/admin/oidc/clients/' + encodeURIComponent(id)); },
    updateOidcClient: function (id, body) { return call('PUT', '/admin/oidc/clients/' + encodeURIComponent(id), body); },
    deleteOidcClient: function (id) { return call('DELETE', '/admin/oidc/clients/' + encodeURIComponent(id)); },
    rotateOidcClientSecret: function (id) {
      return call('POST', '/admin/oidc/clients/' + encodeURIComponent(id) + '/rotate-secret');
    },

    // OIDC upstream providers
    listOidcUpstream: function () { return call('GET', '/admin/oidc/upstream'); },
    upsertOidcUpstream: function (body) { return call('POST', '/admin/oidc/upstream', body); },
    getOidcUpstream: function (slug) { return call('GET', '/admin/oidc/upstream/' + encodeURIComponent(slug)); },
    deleteOidcUpstream: function (slug) {
      return call('DELETE', '/admin/oidc/upstream/' + encodeURIComponent(slug));
    },

    // Zanzibar
    listZanzibarNamespaces: function () { return call('GET', '/admin/auth/zanzibar/namespaces'); },
    getZanzibarNamespace: function (name) {
      return call('GET', '/admin/auth/zanzibar/namespaces/' + encodeURIComponent(name));
    },
    writeZanzibarTuple: function (body) { return call('POST', '/admin/auth/zanzibar/tuples', body); },
    deleteZanzibarTuple: function (body) {
      // DELETE with body is non-standard but supported here for tuple deletion
      const url = '/auth/admin/auth/zanzibar/tuples';
      return fetch(url, {
        method: 'DELETE',
        headers: authHeaders(),
        body: JSON.stringify(body),
      }).then(async function (res) {
        if (res.status === 204) return null;
        const t = await res.text();
        let p; try { p = t ? JSON.parse(t) : null; } catch (_) { p = t; }
        if (!res.ok) {
          const msg = (p && (p.error_description || p.error)) || t || res.statusText;
          throw new Error(msg);
        }
        return p;
      });
    },
    checkZanzibar: function (body) { return call('POST', '/admin/auth/zanzibar/check', body); },
    expandZanzibar: function (body) { return call('POST', '/admin/auth/zanzibar/expand', body); },

    // Keys
    biscuit: function () { return call('GET', '/admin/auth/biscuit'); },
    jwks: function () { return call('GET', '/admin/auth/jwks'); },

    // Audit
    audit: function (params) { return call('GET', '/admin/auth/audit' + qs(params)); },
  };
})();
