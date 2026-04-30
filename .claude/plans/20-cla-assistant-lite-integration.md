# Plan 20 — CLA Assistant Lite integration

**Status:** drafted, executing **Successor of:** none (governance, not feature) **Target tag:** none
(infra change, no version bump)

## Why this exists

`CONTRIBUTING.md` and `CLA.md` both promise that "the CLA Assistant bot will post a comment with a
link" when a contributor opens a PR. No workflow exists to make that true. Either the docs are
aspirational or a SaaS GitHub App is silently installed at the org level — either way, the
contribution gating for the relicense grant in `CLA.md` §2 is not git-tracked and not auditable from
the repo.

We picked **CLA Assistant Lite** (the `contributor-assistant/github-action` action) over the
cla-assistant.io SaaS because:

- Signatures live in this repo (git-tracked, auditable, no third-party SPOF).
- No external GitHub App permissions granted to a third-party site.
- Fits assay's "everything in one binary / one repo" ethos.

## Architecture

- **Action**: `contributor-assistant/github-action@v2.6.1`, pinned to a tag.
- **Triggers**: `pull_request_target` (opened, closed, synchronize) + `issue_comment` (created).
- **Signature storage**: dedicated **orphan branch** `cla-signatures` inside the assay repo, file at
  `signatures/version1/cla.json`. main is branch-protected (`ci.yml` comment confirms direct pushes
  are blocked), so storing on main would require a branch-protection exception or a PAT with bypass.
  An orphan branch sidesteps both and keeps signatures cleanly separated from project history.
- **Token**: built-in `GITHUB_TOKEN` is sufficient (same-repo storage; the action's
  `PERSONAL_ACCESS_TOKEN` is only needed when storing signatures in a different repo).
- **Magic phrase to sign**: `I have read the CLA Document and I hereby sign the CLA` (action
  default, referenced verbatim in updated docs).
- **Allowlist**: `dependabot[bot]`, `renovate[bot]`, `github-actions[bot]`, `developerinlondon`
  (project owner — exempt from self-signing).
- **Versioning**: `path-to-signatures` includes `version1`. When `CLA.md` materially changes, bump
  to `version2`; existing signers will be re-prompted (old signatures remain in `version1/cla.json`
  for audit).

## Tech Stack

- GitHub Actions (workflow YAML)
- `contributor-assistant/github-action` v2.6.1
- dprint for markdown / yaml / json formatting (per `dprint.json`)

## Decisions locked

- **Storage location**: dedicated `cla-signatures` orphan branch, **not** main, **not** a separate
  repo, **not** the SaaS.
- **Action version**: pin to `v2.6.1` (full semver tag), not `v2`. Upgrades go through PR review.
- **Allowlist**: bots only + project owner. No human collaborator allowlists.
- **Branch protection**: main gains a required status check `license/cla` once the action runs once
  and registers the check name with GitHub.
- **Worktree**: not used. Work happens on `feature/cla-assistant-lite` branch in
  `/home/eda/code/assay/`.

## File map

| File                                                     | Action    | Responsibility                                  |
| -------------------------------------------------------- | --------- | ----------------------------------------------- |
| `signatures/version1/cla.json` (on `cla-signatures` br.) | create    | Seed signatures store                           |
| `.github/workflows/cla.yml`                              | create    | Action wiring                                   |
| `CLA.md`                                                 | modify    | Replace §"How to sign" with the actual flow     |
| `CONTRIBUTING.md`                                        | modify    | Replace CLA paragraph with the actual flow      |
| `README.md`                                              | modify    | Add CLA badge near other badges                 |
| Branch protection rule on `main` (GitHub UI)             | configure | Require `license/cla` status check before merge |

---

## Task 1 — Create the `cla-signatures` orphan branch with seed file

**Files:**

- Create: `signatures/version1/cla.json` (on branch `cla-signatures` only)

**Why orphan:** the branch has no shared history with main; it carries one file and nothing else.
Trivial diff, trivial restore.

- [ ] **Step 1.1: Confirm clean working tree and on main**

```bash
cd /home/eda/code/assay
git status
git rev-parse --abbrev-ref HEAD          # expect: main
```

- [ ] **Step 1.2: Create the orphan branch**

```bash
git checkout --orphan cla-signatures
git rm -rf .                             # remove all tracked files from the index
```

- [ ] **Step 1.3: Write the seed signatures file**

```bash
mkdir -p signatures/version1
printf '{\n  "signedContributors": []\n}\n' > signatures/version1/cla.json
```

- [ ] **Step 1.4: Commit on the orphan branch**

```bash
git add signatures/version1/cla.json
git commit -m "chore(cla): seed empty signatures store on cla-signatures branch"
```

- [ ] **Step 1.5: PAUSE — confirm with user before push**

Push command (run only after user confirms):

```bash
git push -u origin cla-signatures
```

- [ ] **Step 1.6: Return to main**

```bash
git checkout main
```

---

## Task 2 — Add the CLA workflow

**Files:**

- Create: `.github/workflows/cla.yml`

- [ ] **Step 2.1: Create the feature branch**

```bash
git checkout -b feature/cla-assistant-lite
```

- [ ] **Step 2.2: Write the workflow file**

Content of `.github/workflows/cla.yml`:

```yaml
name: CLA Assistant

on:
  issue_comment:
    types: [created]
  pull_request_target:
    types: [opened, closed, synchronize]

permissions:
  actions: write
  contents: write
  pull-requests: write
  statuses: write

jobs:
  cla:
    runs-on: ubuntu-latest
    if: |
      github.event_name == 'pull_request_target'
        || github.event.comment.body == 'recheck'
        || github.event.comment.body == 'I have read the CLA Document and I hereby sign the CLA'
    steps:
      - name: CLA Assistant
        uses: contributor-assistant/github-action@v2.6.1
        env:
          GITHUB_TOKEN: ${{ secrets.GITHUB_TOKEN }}
        with:
          path-to-document: https://github.com/developerinlondon/assay/blob/main/CLA.md
          path-to-signatures: signatures/version1/cla.json
          branch: cla-signatures
          allowlist: developerinlondon,dependabot[bot],renovate[bot],github-actions[bot]
          custom-notsigned-prcomment: |
            Thank you for your contribution to **assay**. Before this PR can be merged, please
            sign the [Contributor License Agreement](https://github.com/developerinlondon/assay/blob/main/CLA.md).

            **To sign**, post a new comment on this PR containing exactly the line below:
          custom-pr-sign-comment: I have read the CLA Document and I hereby sign the CLA
          custom-allsigned-prcomment: All contributors have signed the CLA. Thank you.
          lock-pullrequest-aftermerge: true
          create-file-commit-message: "chore(cla): create signatures file"
          signed-commit-message: "chore(cla): @$contributorName signed CLA in $owner/$repo#$pullRequestNo"
```

- [ ] **Step 2.3: Validate YAML syntax**

```bash
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/cla.yml'))" && echo valid
```

- [ ] **Step 2.4: dprint fmt**

```bash
dprint fmt .github/workflows/cla.yml
```

- [ ] **Step 2.5: Commit**

```bash
git add .github/workflows/cla.yml
git commit -m "ci(cla): add CLA Assistant Lite workflow"
```

---

## Task 3 — Update `CLA.md` §"How to sign"

**Files:**

- Modify: `CLA.md:94-102`

- [ ] **Step 3.1: Replace the existing section**

Old (`CLA.md:94-102`):

```markdown
## How to sign

When You open a pull request, the **CLA Assistant** bot will post a comment asking You to sign this
Agreement. You sign by clicking the link in the bot's comment and agreeing to these terms with Your
GitHub account. You only need to sign once — the signature applies to all Your future Contributions
to the Project.

If You are contributing on behalf of an employer, please ensure You have Your employer's permission
before signing.
```

New:

```markdown
## How to sign

When You open a pull request, the **CLA Assistant** workflow posts a comment listing the
contributors on the PR who have not yet signed this Agreement. You sign by adding a new comment to
the PR containing exactly this single line:

> I have read the CLA Document and I hereby sign the CLA

The workflow records Your GitHub username and signing timestamp in
[`signatures/version1/cla.json`](https://github.com/developerinlondon/assay/blob/cla-signatures/signatures/version1/cla.json)
on the `cla-signatures` branch of this repository. You only need to sign once — Your signature
applies to all Your future Contributions to the Project at the current Agreement version.

If the Agreement is materially revised, the signatures path will be bumped (e.g.
`signatures/version2/cla.json`) and You will be prompted to sign again. Prior signatures remain in
the previous-version file for audit.

If You are contributing on behalf of an employer, please ensure You have Your employer's permission
before signing.
```

- [ ] **Step 3.2: dprint fmt**

```bash
dprint fmt CLA.md
```

- [ ] **Step 3.3: Commit**

```bash
git add CLA.md
git commit -m "docs(cla): rewrite \"How to sign\" for the magic-phrase flow"
```

---

## Task 4 — Update `CONTRIBUTING.md` CLA section

**Files:**

- Modify: `CONTRIBUTING.md:38-53`

- [ ] **Step 4.1: Replace the existing section**

Old (`CONTRIBUTING.md:38-53`):

```markdown
## Contributor License Agreement (CLA)

Assay requires all contributors to sign a Contributor License Agreement before their PRs can be
merged. The full text of the CLA is in [`CLA.md`](CLA.md).

**Why we have a CLA**: it lets the project owner relicense the project (or include contributions in
proprietary commercial editions) in the future without needing to track down every contributor for
permission. You retain the copyright on your contribution; you grant the project owner a broad
license to use it.

**How to sign**: when you open your first PR, the CLA Assistant bot will post a comment with a link.
Click the link, agree to the terms with your GitHub account, and you're done — your signature is
recorded for all future PRs to this project.

If you can't or won't sign the CLA (for example, because your employer prohibits it), please open an
issue describing the change instead and we'll figure out an alternative path together.
```

New:

```markdown
## Contributor License Agreement (CLA)

Assay requires all contributors to sign a Contributor License Agreement before their PRs can be
merged. The full text of the CLA is in [`CLA.md`](CLA.md).

**Why we have a CLA**: it lets the project owner relicense the project (or include contributions in
proprietary commercial editions) in the future without needing to track down every contributor for
permission. You retain the copyright on your contribution; you grant the project owner a broad
license to use it.

**How to sign**: when you open a PR, the **CLA Assistant** workflow comments on it. To sign, post a
new comment on the PR containing exactly this single line:

> I have read the CLA Document and I hereby sign the CLA

The workflow records your GitHub username and timestamp in
[`signatures/version1/cla.json`](https://github.com/developerinlondon/assay/blob/cla-signatures/signatures/version1/cla.json)
on the `cla-signatures` branch. You only need to sign once — the signature applies to all your
future PRs at the current CLA version.

If you push new commits to a PR after signing, post `recheck` as a comment to re-trigger the
workflow.

If you can't or won't sign the CLA (for example, because your employer prohibits it), please open an
issue describing the change instead and we'll figure out an alternative path together.
```

- [ ] **Step 4.2: dprint fmt and commit**

```bash
dprint fmt CONTRIBUTING.md
git add CONTRIBUTING.md
git commit -m "docs(contributing): describe magic-phrase CLA signing flow"
```

---

## Task 5 — Add CLA badge to `README.md`

**Files:**

- Modify: `README.md` (badge area near top)

- [ ] **Step 5.1: Locate badge area**

```bash
grep -n -E "^\[!\[|shields\.io" README.md | head -5
```

- [ ] **Step 5.2: Add the badge**

Either append to the existing badge row, or insert under the title:

```markdown
[![CLA assistant](https://github.com/developerinlondon/assay/actions/workflows/cla.yml/badge.svg)](https://github.com/developerinlondon/assay/actions/workflows/cla.yml)
```

- [ ] **Step 5.3: dprint fmt and commit**

```bash
dprint fmt README.md
git add README.md
git commit -m "docs(readme): add CLA assistant workflow badge"
```

---

## Task 6 — PAUSE: open the integration PR

Push and open PR only after user confirms.

- [ ] **Step 6.1: Push the feature branch**

```bash
git push -u origin feature/cla-assistant-lite
```

- [ ] **Step 6.2: Open the PR with `--assignee @me`**

```bash
gh pr create --assignee @me \
  --title "ci(cla): wire up CLA Assistant Lite" \
  --body "$(cat <<'EOF'
## Summary

- Adds `.github/workflows/cla.yml` running `contributor-assistant/github-action@v2.6.1`.
- Signatures stored on dedicated `cla-signatures` orphan branch at `signatures/version1/cla.json`.
- Updates `CLA.md` and `CONTRIBUTING.md` "How to sign" sections to match the actual flow
  (magic-phrase comment, not a click-through link).
- Adds CLA badge to `README.md`.

Closes the gap where the docs promised a CLA bot but no workflow existed.

## Test plan

- [ ] CI green on this PR.
- [ ] CLA Assistant comments on this PR requesting signature from the owner.
- [ ] Owner signs via magic-phrase comment; signature recorded in `cla-signatures` branch
      `signatures/version1/cla.json`.
- [ ] Status check `license/cla` (or whatever name the action posts) appears green on this PR.
- [ ] After merge: add the status check name to required checks on `main` via branch protection
      settings (Task 8).
EOF
)"
```

---

## Task 7 — End-to-end verification

- [ ] **Step 7.1: Confirm the workflow ran and posted the not-signed comment**

Within ~30 seconds of PR open, expect a bot comment requesting signature. If absent within 2
minutes:

```bash
gh run list --workflow cla.yml --limit 5
gh run view <run-id> --log
```

Common failure modes:

| Symptom                                         | Cause                                                  | Fix                                                                                                |
| ----------------------------------------------- | ------------------------------------------------------ | -------------------------------------------------------------------------------------------------- |
| `Resource not accessible by integration`        | Default workflow permissions are read-only             | Settings → Actions → General → Workflow permissions → Read and write + tick the PR write checkbox. |
| `branch 'cla-signatures' not found`             | Task 1 push not done                                   | Re-run Task 1.5.                                                                                   |
| `file 'signatures/version1/cla.json' not found` | Wrong branch in workflow                               | Verify `branch: cla-signatures` in `cla.yml`.                                                      |
| Workflow doesn't run                            | First-time `pull_request_target` gating for new actors | Owner-opened PR should fire immediately; if not, push a no-op commit.                              |

- [ ] **Step 7.2: Sign as owner via magic-phrase comment**

```bash
gh pr comment <pr-number> --body "I have read the CLA Document and I hereby sign the CLA"
```

Expected within ~30s: bot edits prior comment to "All contributors have signed the CLA"; new commit
appears on `cla-signatures` adding `developerinlondon` to `signedContributors`.

- [ ] **Step 7.3: Verify recorded signature**

```bash
gh api repos/developerinlondon/assay/contents/signatures/version1/cla.json?ref=cla-signatures \
  --jq '.content' | base64 -d | jq '.signedContributors'
```

- [ ] **Step 7.4: Note the status check name**

In the PR's "Checks" tab, record the exact name of the check the action posts (typically
`license/cla` or `CLA Assistant`). Needed for Task 8.

- [ ] **Step 7.5: Merge the PR**

```bash
gh pr merge --squash
```

---

## Task 8 — PAUSE: lock CLA check into branch protection

Run only after the workflow has executed once on the integration PR and the check name is known.

- [ ] **Step 8.1: Add to required status checks**

GitHub UI: Settings → Branches → branch protection rule for `main` → "Require status checks to pass
before merging" → add the recorded check name.

Or via API (read first, modify, PATCH back):

```bash
gh api repos/developerinlondon/assay/branches/main/protection > /tmp/bp.json
# inspect, then PATCH with the augmented contexts list
```

- [ ] **Step 8.2: Confirm by attempting an unsigned PR**

Open a throwaway test PR from a different account or fork. Verify:

- Bot comment requesting signature.
- Red `license/cla` status.
- Merge button blocked with "Required status check is failing".

Sign via magic-phrase comment, confirm merge unblocks. Close the test PR without merging.

---

## Task 9 — Document the version-bump procedure

**Files:**

- Modify: `CLA.md` (append a short project-owner section)

- [ ] **Step 9.1: Append**

```markdown
## Versioning (project-owner notes)

When this Agreement is materially revised:

1. On the `cla-signatures` branch, copy `signatures/version1/cla.json` to
   `signatures/version2/cla.json`. The `version1` file is preserved untouched as the audit record
   for prior signers.
2. On `main`, bump `path-to-signatures` in `.github/workflows/cla.yml` from
   `signatures/version1/cla.json` to `signatures/version2/cla.json` and commit.
3. Edit the new `signatures/version2/cla.json` to `{ "signedContributors": [] }` so all prior
   signers are re-prompted.
4. Open a PR with the updated CLA text plus the workflow change. Sign it yourself first to validate
   the new path; existing contributors will be prompted on their next PR.
```

- [ ] **Step 9.2: dprint fmt, commit, push, open follow-up PR**

```bash
git checkout main
git pull --ff-only
git checkout -b docs/cla-versioning
dprint fmt CLA.md
git add CLA.md
git commit -m "docs(cla): document version-bump procedure for the project owner"
git push -u origin docs/cla-versioning
gh pr create --assignee @me \
  --title "docs(cla): document CLA version-bump procedure" \
  --body "Adds the project-owner runbook for bumping path-to-signatures when the CLA materially changes."
```

This second PR is also a useful smoke test — it should hit the freshly-installed CLA workflow and
require the owner to sign again under the new branch.

---

## Risks & open questions

- **First-PR `pull_request_target` quirk.** GitHub gates `pull_request_target` for first-time
  contributors. Project-owner-opened PRs are unaffected; external contributors may see no comment
  until a maintainer approves the workflow run once. The action's README acknowledges this.
- **Repo workflow permissions.** If the repo's default workflow permissions are read-only (the
  GitHub default for new repos changed in 2023), the action will fail to commit. Step 7.1
  troubleshooting table covers this.
- **`v2.6.1` may no longer be the latest stable tag.** Check
  `gh release view --repo contributor-assistant/github-action` before Task 2 and bump the pin if
  needed.
