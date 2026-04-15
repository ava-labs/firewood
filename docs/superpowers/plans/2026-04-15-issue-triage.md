# Issue Triage Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to work through this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Triage all 103 open GitHub issues for ava-labs/firewood: categorize, size, identify stale/fixed issues, and produce a single recommendation document for manual execution.

**Architecture:** Paginate issues in batches of ~25 using `gh issue list --json`, accumulate results into a structured Markdown triage document, then cross-reference against recent merged PRs to catch independently-fixed issues.

**Tech Stack:** `gh` CLI (JSON output), Python 3 stdlib for batch scripting, Markdown output document.

---

## Triage Criteria

### Categories (map to existing labels)

| Category | Use When |
|----------|----------|
| `bug` | Describes broken/incorrect behavior |
| `enhancement` | New feature or improvement to existing behavior |
| `documentation` | Docs gap or improvement |
| `performance` | Speed, memory, throughput concern |
| `testing` | Missing or broken tests |
| `observability` | Logging, tracing, metrics |
| `techdebt` / `cleanup` | Code quality, refactor, dead code |
| `devops` / `dev infra` | CI/CD, build system, benchmarking infrastructure |
| `storage` | Storage layer specifics |
| `security` | Security concern |
| `Research Project` | Requires investigation before solution is clear |
| `question` | Needs discussion/decision, not code |

### T-Shirt Sizes

| Size | Meaning |
|------|---------|
| XS | Trivial — < 2 hours, isolated change, clear solution |
| S | Small — half day, one component, clear approach |
| M | Medium — 1–3 days, touches multiple components or needs design |
| L | Large — 1–2 weeks, significant new system or deep change |
| XL | Extra-large — multi-week, major architectural work |

### Recommended Actions

| Action | When to Use |
|--------|-------------|
| `keep` | Issue is valid, still relevant, no changes needed |
| `relabel` | Issue is valid but labels are wrong or missing |
| `close/fixed` | Fixed by a merged PR (cite the PR) |
| `close/stale` | No longer relevant — problem gone, approach obsolete, etc. |
| `close/duplicate` | Duplicate of another open issue (cite the other issue) |
| `close/wontfix` | Valid issue but we've decided not to address it |
| `needs-info` | Can't evaluate without more information |

---

## Output Document

All findings accumulate in: `docs/triage/2026-04-15-issue-triage.md`

Structure:
1. **Summary table** — every issue: number, title, category, size, action
2. **Issues to close** — grouped by reason, with rationale
3. **Label changes** — per issue: labels to add/remove
4. **Priority picks** — top issues worth scheduling next sprint

---

## File Layout

- Read: none (all data from `gh`)
- Write: `docs/triage/2026-04-15-issue-triage.md` (output)
- Temp: `/tmp/fw-issues-*.json` (batch fetches, discarded after)

---

## Task 1: Setup — fetch all issue numbers and seed the output document

**Files:**
- Write: `docs/triage/2026-04-15-issue-triage.md`
- Temp: `/tmp/fw-issues-all.json`

- [ ] **Step 1: Fetch all open issue numbers with lightweight metadata**

```bash
gh issue list --repo ava-labs/firewood --state open \
  --json number,title,labels,createdAt,updatedAt \
  --limit 200 > /tmp/fw-issues-all.json
python3 -c "import json; d=json.load(open('/tmp/fw-issues-all.json')); print(len(d))"
```

Expected: prints `103` (or current count)

- [ ] **Step 2: Fetch recently merged PRs that closed issues**

```bash
gh pr list --repo ava-labs/firewood --state merged \
  --json number,title,body,mergedAt \
  --limit 100 > /tmp/fw-merged-prs.json
# Grep for issue references in PR bodies
python3 -c "
import json, re
prs = json.load(open('/tmp/fw-merged-prs.json'))
closes = {}
for pr in prs:
    body = pr.get('body') or ''
    nums = re.findall(r'(?:closes?|fixes?|resolves?)\s+#(\d+)', body, re.IGNORECASE)
    for n in nums:
        closes[int(n)] = pr['number']
for issue_num in sorted(closes):
    print(f'  Issue #{issue_num} closed by PR #{closes[issue_num]}')
print(f'Total: {len(closes)} issues auto-closed by merged PRs')
"
```

- [ ] **Step 3: Create the output document with headers**

Create `docs/triage/2026-04-15-issue-triage.md` (see output format below).

- [ ] **Step 4: Commit the empty scaffold**

```bash
git add docs/triage/2026-04-15-issue-triage.md
git commit -m "docs(triage): add issue triage scaffold for 2026-04-15"
```

---

## Task 2: Batch 1 — Issues sorted oldest-to-newest, first ~26

**Files:**
- Modify: `docs/triage/2026-04-15-issue-triage.md`
- Temp: `/tmp/fw-issues-batch1.json`

- [ ] **Step 1: Fetch batch 1 with full body**

```bash
gh issue list --repo ava-labs/firewood --state open \
  --json number,title,body,labels,createdAt,updatedAt,comments \
  --limit 26 --order asc \
  > /tmp/fw-issues-batch1.json
```

- [ ] **Step 2: For each issue in the batch, read body and evaluate**

For each issue, determine:
1. Is this still a valid, unaddressed problem? Check the codebase if needed (`grep`, `Glob`).
2. What category best fits?
3. What t-shirt size?
4. What action is recommended?

- [ ] **Step 3: Append batch 1 findings to the triage document**

Append rows to the summary table and notes to the relevant sections.

- [ ] **Step 4: Commit batch 1 results**

```bash
git add docs/triage/2026-04-15-issue-triage.md
git commit -m "docs(triage): add batch 1 issue analysis"
```

---

## Task 3: Batch 2 — next ~26 issues

Same steps as Task 2 with:
```bash
gh issue list --repo ava-labs/firewood --state open \
  --json number,title,body,labels,createdAt,updatedAt,comments \
  --limit 26 --offset 26 --order asc \
  > /tmp/fw-issues-batch2.json
```

Commit: `docs(triage): add batch 2 issue analysis`

---

## Task 4: Batch 3 — next ~26 issues

Same steps as Task 2 with:
```bash
gh issue list --repo ava-labs/firewood --state open \
  --json number,title,body,labels,createdAt,updatedAt,comments \
  --limit 26 --offset 52 --order asc \
  > /tmp/fw-issues-batch3.json
```

Commit: `docs(triage): add batch 3 issue analysis`

---

## Task 5: Batch 4 — remaining issues

Same steps as Task 2 with:
```bash
gh issue list --repo ava-labs/firewood --state open \
  --json number,title,body,labels,createdAt,updatedAt,comments \
  --limit 50 --offset 78 --order asc \
  > /tmp/fw-issues-batch4.json
```

Commit: `docs(triage): add batch 4 issue analysis`

---

## Task 6: Cross-reference and stale detection

**Files:**
- Modify: `docs/triage/2026-04-15-issue-triage.md`

- [ ] **Step 1: Check each issue marked `close/fixed` against the merged PRs list**

Confirm the referenced PR is actually merged and the issue is actually resolved.

- [ ] **Step 2: Scan for issues referencing removed APIs or paths**

For any issue that references a specific function, file, or module, verify it still exists:
```bash
# Example for each suspect issue
grep -r "function_name" /Users/brandon.leblanc/src/firewood/
```

- [ ] **Step 3: Review age-based stale candidates**

Issues older than 12 months with no comments and no recent activity are candidates for `close/stale`. Flag them.

- [ ] **Step 4: Commit cross-reference updates**

```bash
git add docs/triage/2026-04-15-issue-triage.md
git commit -m "docs(triage): add cross-reference and stale detection results"
```

---

## Task 7: Finalize — compile priority picks and review summary

**Files:**
- Modify: `docs/triage/2026-04-15-issue-triage.md`

- [ ] **Step 1: Identify top 10 priority issues**

From the `keep` issues, identify the highest-value work:
- Bugs with known reproduction
- Enhancements with clear scope (S or M size) that unblock users
- Quick wins (XS/S) suitable for a sprint

- [ ] **Step 2: Write the "Priority Picks" section**

Add a section at the top of the document with a table of the top 10.

- [ ] **Step 3: Write the triage summary statistics**

```
Total issues reviewed: 103
- Keep (no change): N
- Relabel: N
- Close/fixed: N
- Close/stale: N
- Close/duplicate: N
- Close/wontfix: N
- Needs info: N
```

- [ ] **Step 4: Final commit**

```bash
git add docs/triage/2026-04-15-issue-triage.md
git commit -m "docs(triage): finalize 2026-04-15 issue triage document"
```

---

## Output Document Template

````markdown
# Firewood Issue Triage — 2026-04-15

> Generated by issue triage process. All changes are recommendations — execute manually on GitHub.

## Triage Stats

<!-- Fill in after all batches complete -->
- Total open issues reviewed: 103
- Keep (no change needed): 
- Relabel only: 
- Close/fixed by PR: 
- Close/stale: 
- Close/duplicate: 
- Close/wontfix: 
- Needs more info: 

## Priority Picks (Top Issues to Schedule)

| # | Title | Category | Size | Why Priority |
|---|-------|----------|------|--------------|

---

## Summary Table

| # | Title | Current Labels | Category | Size | Action |
|---|-------|----------------|----------|------|--------|

---

## Issues to Close

### Fixed by Merged PR
<!-- Format: - #NNN: Title — fixed by PR #NNN (brief note) -->

### No Longer Relevant / Stale
<!-- Format: - #NNN: Title — reason -->

### Duplicates
<!-- Format: - #NNN: Title — duplicate of #NNN -->

### Won't Fix
<!-- Format: - #NNN: Title — reason -->

---

## Label Changes

### Add Labels
<!-- Format: - #NNN: add `label-name` -->

### Remove Labels
<!-- Format: - #NNN: remove `label-name` -->

### Replace Labels
<!-- Format: - #NNN: replace `old-label` with `new-label` -->

---

## Issue Body Updates Needed

<!-- For issues where the description is outdated or unclear -->
<!-- Format: - #NNN: [brief description of what needs updating] -->

---

## Notes & Observations

<!-- Patterns noticed, systemic issues, suggestions for future process -->
````
