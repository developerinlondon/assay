/* Assay Auth — login landing controller.
 *
 * Fetches enabled upstream IdPs from /auth/upstreams and renders one
 * button per row, preserving the `return_to` query parameter so the
 * user lands back at the page that bounced them here once the
 * upstream round-trip completes.
 *
 * Provider icons come from /auth/icons.svg (a single sprite shipped
 * with the auth dashboard). The button references the right symbol by
 * slug — Google/GitHub/GitLab/Microsoft/Apple/Discord/Slack are mapped
 * to their Simple Icons paths; everything else falls back to a generic
 * shield-and-person glyph. Operators can override per-upstream by
 * setting icon_url on the upstream row.
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

  const SVG_NS = 'http://www.w3.org/2000/svg';
  const XLINK_NS = 'http://www.w3.org/1999/xlink';

  // Map an upstream to a sprite symbol id. Substring match on slug +
  // display_name so a provider named "Corporate Google Workspace" still
  // gets the Google icon. Returns 'generic' for anything unmatched.
  function spriteIdFor(slug, displayName) {
    const key = ((slug || '') + ' ' + (displayName || '')).toLowerCase();
    if (key.indexOf('google') !== -1) return 'google';
    if (key.indexOf('github') !== -1) return 'github';
    if (key.indexOf('gitlab') !== -1) return 'gitlab';
    if (key.indexOf('microsoft') !== -1 || key.indexOf('azure') !== -1 || key.indexOf('entra') !== -1) return 'microsoft';
    if (key.indexOf('apple') !== -1) return 'apple';
    if (key.indexOf('discord') !== -1) return 'discord';
    if (key.indexOf('slack') !== -1) return 'slack';
    return 'generic';
  }

  function makeSpriteIcon(symbolId) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('width', '20');
    svg.setAttribute('height', '20');
    svg.setAttribute('aria-hidden', 'true');
    const use = document.createElementNS(SVG_NS, 'use');
    // Set both `href` (modern) and `xlink:href` (legacy) — same-doc
    // references work without either on modern browsers, but external
    // sprite refs need `href` and some user agents only honour xlink.
    use.setAttribute('href', '/auth/icons.svg#' + symbolId);
    use.setAttributeNS(XLINK_NS, 'xlink:href', '/auth/icons.svg#' + symbolId);
    svg.appendChild(use);
    return svg;
  }

  // Brand-tinted hover class names. Same lookup as the sprite id.
  function brandClassFor(slug, displayName) {
    const key = ((slug || '') + ' ' + (displayName || '')).toLowerCase();
    if (key.indexOf('google') !== -1) return 'is-google';
    if (key.indexOf('github') !== -1) return 'is-github';
    if (key.indexOf('gitlab') !== -1) return 'is-gitlab';
    if (key.indexOf('microsoft') !== -1 || key.indexOf('azure') !== -1 || key.indexOf('entra') !== -1) return 'is-microsoft';
    if (key.indexOf('apple') !== -1) return 'is-apple';
    if (key.indexOf('discord') !== -1) return 'is-discord';
    if (key.indexOf('slack') !== -1) return 'is-slack';
    return null;
  }

  function showStatus(text, isError) {
    container.dataset.state = isError ? 'error' : 'empty';
    container.innerHTML = '';
    const p = document.createElement('p');
    p.className = 'login-status' + (isError ? ' login-status-error' : '');
    p.textContent = text;
    container.appendChild(p);
  }

  // Same-origin fetch — must allow cookies so the browser attaches the
  // Cloudflare Access cookie (or any perimeter cookie) when this page
  // is loaded through such a gate. `credentials: 'omit'` would strip
  // those and the upstream call gets bounced to the CF Access login.
  fetch('/auth/upstreams', { credentials: 'same-origin' })
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
        const brandClass = brandClassFor(u.slug, u.display_name);
        if (brandClass) a.classList.add(brandClass);

        // Icon resolution:
        //   1. Operator-configured icon_url (raster or SVG asset)
        //   2. Sprite symbol from /auth/icons.svg keyed on provider
        //      (falls back to the 'generic' symbol for unknowns)
        const iconWrap = document.createElement('span');
        iconWrap.className = 'login-button-icon';
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
          iconWrap.appendChild(img);
        } else {
          iconWrap.appendChild(makeSpriteIcon(spriteIdFor(u.slug, u.display_name)));
        }
        a.appendChild(iconWrap);

        const label = document.createElement('span');
        label.className = 'login-button-label';
        label.textContent = 'Sign in with ' + (u.display_name || u.slug);
        a.appendChild(label);
        container.appendChild(a);
      });
    })
    .catch(function () {
      showStatus('Could not load sign-in options.', true);
    });
})();
