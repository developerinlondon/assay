---
name: clickup
description: Working knowledge for driving ClickUp well through assay.clickup or the raw v2 API — rich descriptions, Quill comments, status mechanics, tags, dates, subtasks, and the auth quirk. Load before creating or editing ClickUp tasks, epics, or comments.
metadata:
  author: developerinlondon
  version: "0.1.0"
---

# ClickUp, driven properly

The v2 API is broad — tasks, subtasks, tags, dates, priorities, custom fields, attachments,
comments, goals, time tracking all work. What follows is the working knowledge that keeps an
agent from producing lumped-text tasks and silent formatting loss. Prefer `assay.clickup`
(`require("assay.clickup")`) over hand-rolled HTTP; everything here applies to both.

## Auth

Personal `pk_` tokens go in `Authorization` **raw** — a `Bearer ` prefix is rejected. Read the
token from the vault (`platform/clickup`), never from a file in the repo.

## Rich text: which field, which surface

| Surface | Format | Rule |
| --- | --- | --- |
| Task description | **`markdown_description`** on create AND update | Full markdown renders: headings, bold, lists, links, tables. The plain `description` field is lumped text — never write it when structure matters. |
| Reading back | `?include_markdown_description=true` | The default `text_content`/`description` strips formatting — round-tripping through them and PUTting back DESTROYS the markdown. Always read the markdown field, edit that, write it back. |
| Comments | **Quill delta, not markdown** | `comment_text` renders literally — `**bold**` shows asterisks. Keep comments to plain sentences, or build a `comment` array of Quill ops for real formatting. |

## Statuses

- The API **cannot create or edit statuses** — `POST/PUT /space` silently ignores a `statuses`
  payload; API-created spaces get only `to do`/`complete`. A human adds statuses in Space
  settings once; after that the API sets them freely (`status` on task update).
- Until custom statuses exist, carry state as **tags** (`in-flight`, `in-review`,
  `blocked-…`) and say so. Never demote a task to a wrong status to work around a missing one.

## The fields that make a task legible

Fill these at creation, not as an afterthought: `due_date`/`start_date` (ms epoch UTC),
`priority` (1 urgent → 4 low), `tags` (create-on-attach via `POST /task/{id}/tag/{name}`),
`assignees` (user ids from `GET /team`), `parent` for subtasks (same list), `time_estimate`
(ms). Attachments upload as multipart to `/task/{id}/attachment` — mockups belong ON the task.

## Blocked means blocked

Tag work `blocked-…` only when nothing in it can proceed. Key-gated integrations are
**buildable against published API docs and fixtures** — only the live smoke test waits for
the key, so the ticket is `in-flight` with a note, not blocked.

## Workspace facts (this estate)

Workspace `NSM Technologies Ltd` = team id `90182966627`; v2 calls a workspace a "team".
Rate limit ~100 req/min per token — batch reads, don't poll.
