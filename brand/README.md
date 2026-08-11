# Brand

| File                  | Use                                                 |
| --------------------- | --------------------------------------------------- |
| `assay-logo-512.png`  | source-resolution square mark                       |
| `assay-logo-120.png`  | Google OAuth consent screen (120x120 is the minimum) |

Palette: `#e6662a` on `#0d1117` — the accent and surface the auth UI already uses.

Google's consent screen takes a square JPG/PNG/BMP of at least 120x120 and under 1 MB.
Uploading one moves the project into Google's brand-verification queue, so an app that
only needs `openid`/`email`/`profile` can ship without ever setting it.
