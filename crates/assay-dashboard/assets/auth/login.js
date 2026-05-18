/* Assay Auth — login landing controller.
 *
 * Fetches enabled upstream IdPs from /auth/upstreams and renders one
 * button per row, preserving the `return_to` query parameter so the
 * user lands back at the page that bounced them here once the
 * upstream round-trip completes.
 *
 * DOM-build only — never innerHTML with provider-supplied fields
 * (display_name, icon_url). textContent + element properties are
 * XSS-safe by construction.
 */

(function () {
  'use strict';

  const params = new URLSearchParams(window.location.search);
  const returnTo = params.get('return_to') || '/';

  const container = document.getElementById('upstreams');
  if (!container) return;

  function showStatus(text, isError) {
    container.dataset.state = isError ? 'error' : 'empty';
    container.innerHTML = '';
    const p = document.createElement('p');
    p.className = 'login-status' + (isError ? ' login-status-error' : '');
    p.textContent = text;
    container.appendChild(p);
  }

  fetch('/auth/upstreams', { credentials: 'omit' })
    .then(function (r) {
      if (!r.ok) throw new Error('http ' + r.status);
      return r.json();
    })
    .then(function (upstreams) {
      if (!Array.isArray(upstreams) || upstreams.length === 0) {
        showStatus('No sign-in providers configured.');
        return;
      }
      container.dataset.state = 'ready';
      container.innerHTML = '';
      upstreams.forEach(function (u) {
        const a = document.createElement('a');
        a.className = 'login-button';
        a.href = '/auth/oidc/upstream/' + encodeURIComponent(u.slug)
          + '/start?return_to=' + encodeURIComponent(returnTo);
        a.dataset.slug = u.slug;

        if (u.icon_url) {
          const img = document.createElement('img');
          // `no-referrer` keeps the Referer header off the icon
          // fetch — otherwise a third-party CDN hosting the icon
          // would see `/auth/login?return_to=...` (OIDC request
          // metadata) in its logs.
          img.referrerPolicy = 'no-referrer';
          img.src = u.icon_url;
          img.alt = '';
          img.width = 20;
          img.height = 20;
          a.appendChild(img);
        }
        const label = document.createElement('span');
        label.textContent = 'Sign in with ' + (u.display_name || u.slug);
        a.appendChild(label);
        container.appendChild(a);
      });
    })
    .catch(function () {
      showStatus('Could not load sign-in options.', true);
    });
})();
