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

  const SVG_NS = 'http://www.w3.org/2000/svg';

  function makeSvg(viewBox, paths) {
    const svg = document.createElementNS(SVG_NS, 'svg');
    svg.setAttribute('viewBox', viewBox);
    svg.setAttribute('width', '20');
    svg.setAttribute('height', '20');
    svg.setAttribute('aria-hidden', 'true');
    paths.forEach(function (p) {
      const el = document.createElementNS(SVG_NS, 'path');
      Object.keys(p).forEach(function (k) { el.setAttribute(k, p[k]); });
      svg.appendChild(el);
    });
    return svg;
  }

  /* Built-in brand icons for well-known upstream providers.
   * Keyed by slug substring (case-insensitive). When an operator has
   * not configured an `icon_url`, this gives Google / GitHub / etc. a
   * recognizable mark without needing a third-party asset host.
   * Returns an SVGElement or null. */
  function builtinIcon(slug, displayName) {
    const key = ((slug || '') + ' ' + (displayName || '')).toLowerCase();
    if (key.indexOf('google') !== -1) {
      return makeSvg('0 0 48 48', [
        { d: 'M44.5 20H24v8.5h11.8C34.7 33.9 30.1 37 24 37c-7.2 0-13-5.8-13-13s5.8-13 13-13c3.1 0 5.9 1.1 8.1 2.9l6.4-6.4C34.6 4.1 29.6 2 24 2 11.8 2 2 11.8 2 24s9.8 22 22 22c11 0 21-8 21-22 0-1.3-.2-2.7-.5-4z', fill: '#FFC107' },
        { d: 'M6.3 14.7l7 5.1C15.2 16.1 19.3 13 24 13c3.1 0 5.9 1.1 8.1 2.9l6.4-6.4C34.6 6.1 29.6 4 24 4 16.3 4 9.7 8.3 6.3 14.7z', fill: '#FF3D00' },
        { d: 'M24 46c5.6 0 10.6-2.1 14.4-5.6l-6.6-5.6c-2 1.4-4.7 2.2-7.8 2.2-6 0-11-3.7-12.9-8.9l-7 5.4C7.7 41.4 15.3 46 24 46z', fill: '#4CAF50' },
        { d: 'M44.5 20H24v8.5h11.8c-.9 2.5-2.5 4.7-4.7 6.3l6.6 5.6C42 36.3 45 30.7 45 24c0-1.3-.2-2.7-.5-4z', fill: '#1976D2' },
      ]);
    }
    if (key.indexOf('github') !== -1) {
      return makeSvg('0 0 24 24', [
        { d: 'M12 0C5.37 0 0 5.37 0 12c0 5.3 3.44 9.8 8.21 11.39.6.11.82-.26.82-.58 0-.29-.01-1.04-.02-2.05-3.34.72-4.04-1.61-4.04-1.61-.55-1.39-1.34-1.76-1.34-1.76-1.09-.74.08-.73.08-.73 1.21.08 1.85 1.24 1.85 1.24 1.07 1.83 2.81 1.3 3.5 1 .11-.78.42-1.31.76-1.61-2.67-.3-5.47-1.33-5.47-5.93 0-1.31.47-2.38 1.24-3.22-.13-.31-.54-1.53.12-3.19 0 0 1.01-.32 3.3 1.23a11.5 11.5 0 016 0c2.29-1.55 3.3-1.23 3.3-1.23.66 1.66.25 2.88.12 3.19.77.84 1.24 1.91 1.24 3.22 0 4.61-2.81 5.63-5.49 5.92.43.37.81 1.1.81 2.22 0 1.61-.01 2.9-.01 3.3 0 .32.22.7.83.58A12 12 0 0024 12c0-6.63-5.37-12-12-12z', fill: 'currentColor' },
      ]);
    }
    if (key.indexOf('gitlab') !== -1) {
      return makeSvg('0 0 32 32', [
        { d: 'M31.5 18.5L30 12c-.04-.16-.13-.3-.27-.4a.51.51 0 00-.6.04L25 16l-3-9.5a.5.5 0 00-.95 0L19 12H13l-2-5.5a.5.5 0 00-.95 0L7 16l-4.13-4.36a.51.51 0 00-.6-.04c-.14.1-.23.24-.27.4L.5 18.5a.5.5 0 00.2.55L16 30l15.3-10.95a.5.5 0 00.2-.55z', fill: '#FC6D26' },
        { d: 'M16 30L7 14l-4.5 4.5L16 30z', fill: '#E24329' },
        { d: 'M16 30l9-16 4.5 4.5L16 30z', fill: '#E24329' },
      ]);
    }
    if (key.indexOf('microsoft') !== -1 || key.indexOf('azure') !== -1 || key.indexOf('entra') !== -1 || key.indexOf('aad') !== -1) {
      return makeSvg('0 0 23 23', [
        { d: 'M1 1h10v10H1z', fill: '#F25022' },
        { d: 'M12 1h10v10H12z', fill: '#7FBA00' },
        { d: 'M1 12h10v10H1z', fill: '#00A4EF' },
        { d: 'M12 12h10v10H12z', fill: '#FFB900' },
      ]);
    }
    if (key.indexOf('apple') !== -1) {
      return makeSvg('0 0 24 24', [
        { d: 'M17.04 12.55c.02 2.06 1.81 2.74 1.83 2.75-.02.05-.29.99-.96 1.96-.58.85-1.18 1.69-2.13 1.71-.94.02-1.24-.55-2.31-.55-1.07 0-1.4.53-2.29.57-.92.04-1.62-.92-2.2-1.77-1.19-1.73-2.1-4.87-.88-7 .6-1.05 1.69-1.71 2.86-1.73.9-.02 1.76.61 2.31.61.55 0 1.6-.75 2.69-.64.46.02 1.74.19 2.56 1.4-.07.04-1.53.9-1.51 2.69zM15.27 5.37c.48-.59.81-1.4.72-2.21-.7.03-1.55.47-2.05 1.05-.45.52-.85 1.36-.74 2.15.79.06 1.59-.4 2.07-.99z', fill: 'currentColor' },
      ]);
    }
    if (key.indexOf('discord') !== -1) {
      return makeSvg('0 0 24 24', [
        { d: 'M20.32 4.65a19.79 19.79 0 00-4.89-1.52.07.07 0 00-.08.04c-.21.38-.45.87-.61 1.25a18.27 18.27 0 00-5.49 0c-.16-.39-.4-.87-.62-1.25a.08.08 0 00-.08-.04c-1.7.3-3.36.81-4.89 1.52a.06.06 0 00-.03.03C.5 9.05-.32 13.32.09 17.53a.08.08 0 00.03.05 19.9 19.9 0 005.99 3.03.08.08 0 00.09-.03c.46-.63.87-1.3 1.22-2a.08.08 0 00-.04-.11 13.1 13.1 0 01-1.87-.89.08.08 0 01-.01-.13c.13-.1.25-.2.37-.3a.08.08 0 01.07-.01c3.93 1.79 8.18 1.79 12.06 0a.08.08 0 01.07.01c.12.1.24.2.37.3a.08.08 0 010 .13c-.6.35-1.23.65-1.87.89a.08.08 0 00-.04.11c.36.7.77 1.37 1.22 2a.08.08 0 00.09.03 19.84 19.84 0 005.99-3.03.08.08 0 00.03-.05c.5-4.87-.81-9.1-3.43-12.85a.06.06 0 00-.03-.03zM8.02 14.97c-1.18 0-2.16-1.08-2.16-2.41 0-1.33.96-2.41 2.16-2.41 1.21 0 2.18 1.09 2.16 2.41 0 1.33-.96 2.41-2.16 2.41zm7.97 0c-1.18 0-2.16-1.08-2.16-2.41 0-1.33.96-2.41 2.16-2.41 1.21 0 2.18 1.09 2.16 2.41 0 1.33-.95 2.41-2.16 2.41z', fill: '#5865F2' },
      ]);
    }
    if (key.indexOf('slack') !== -1) {
      return makeSvg('0 0 270 270', [
        { d: 'M99.4 151.2c0 7.1-5.8 12.9-12.9 12.9-7.1 0-12.9-5.8-12.9-12.9 0-7.1 5.8-12.9 12.9-12.9h12.9v12.9zm6.5 0c0-7.1 5.8-12.9 12.9-12.9 7.1 0 12.9 5.8 12.9 12.9v32.3c0 7.1-5.8 12.9-12.9 12.9-7.1 0-12.9-5.8-12.9-12.9v-32.3z', fill: '#E01E5A' },
        { d: 'M118.8 99.4c-7.1 0-12.9-5.8-12.9-12.9 0-7.1 5.8-12.9 12.9-12.9 7.1 0 12.9 5.8 12.9 12.9v12.9h-12.9zm0 6.5c7.1 0 12.9 5.8 12.9 12.9 0 7.1-5.8 12.9-12.9 12.9H86.5c-7.1 0-12.9-5.8-12.9-12.9 0-7.1 5.8-12.9 12.9-12.9h32.3z', fill: '#36C5F0' },
        { d: 'M170.6 118.8c0-7.1 5.8-12.9 12.9-12.9 7.1 0 12.9 5.8 12.9 12.9 0 7.1-5.8 12.9-12.9 12.9h-12.9v-12.9zm-6.5 0c0 7.1-5.8 12.9-12.9 12.9-7.1 0-12.9-5.8-12.9-12.9V86.5c0-7.1 5.8-12.9 12.9-12.9 7.1 0 12.9 5.8 12.9 12.9v32.3z', fill: '#2EB67D' },
        { d: 'M151.2 170.6c7.1 0 12.9 5.8 12.9 12.9 0 7.1-5.8 12.9-12.9 12.9-7.1 0-12.9-5.8-12.9-12.9v-12.9h12.9zm0-6.5c-7.1 0-12.9-5.8-12.9-12.9 0-7.1 5.8-12.9 12.9-12.9h32.3c7.1 0 12.9 5.8 12.9 12.9 0 7.1-5.8 12.9-12.9 12.9h-32.3z', fill: '#ECB22E' },
      ]);
    }
    return null;
  }

  /* Generic identity icon for unknown providers — a simple person-in-
   * shield outline. Uses currentColor so it picks up the button's
   * text colour (theme-aware). */
  function genericIcon() {
    return makeSvg('0 0 24 24', [
      { d: 'M12 2 4 5v6c0 5 3.5 9.5 8 11 4.5-1.5 8-6 8-11V5l-8-3z', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5', 'stroke-linejoin': 'round' },
      { d: 'M12 11.5a2.25 2.25 0 100-4.5 2.25 2.25 0 000 4.5z', fill: 'currentColor' },
      { d: 'M8 16.25c0-1.8 1.8-3 4-3s4 1.2 4 3', fill: 'none', stroke: 'currentColor', 'stroke-width': '1.5', 'stroke-linecap': 'round' },
    ]);
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

        // Icon resolution order:
        //   1. Operator-configured icon_url (raster or SVG asset)
        //   2. Built-in brand SVG for well-known providers
        //   3. Generic identity glyph
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
          iconWrap.appendChild(builtinIcon(u.slug, u.display_name) || genericIcon());
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

  // Map a provider to a brand-tinted button class. Same lookup pattern
  // as builtinIcon — slug or display_name substring match.
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
})();
