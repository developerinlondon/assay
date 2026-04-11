## template

Jinja2-compatible template rendering. No `require()` needed.

- `template.render(path, vars)` → string — Render template file with variables
- `template.render_string(tmpl, vars)` → string — Render template string with variables
  - Supports: `{{ var }}`, `{% for %}`, `{% if %}`, `{% include %}`, filters
