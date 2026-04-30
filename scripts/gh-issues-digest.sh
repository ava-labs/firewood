#!/usr/bin/env bash
set -euo pipefail

# --- Defaults ---
REPO=""
OUTPUT=""
LIMIT=500
INCLUDE_BODY=true
INCLUDE_COMMENTS=true

# --- Help ---
usage() {
    cat <<'HELP'
Usage: gh-issues-digest.sh [options]

Enumerate open GitHub issues and produce a markdown digest for AI agent
triage, categorization, sizing, and prioritization.

Options:
  -r, --repo OWNER/REPO   Repository (default: auto-detect via gh)
  -o, --output FILE        Output file (default: stdout)
  -l, --limit N            Max issues to fetch (default: 500)
  --no-body                Omit issue bodies
  --no-comments            Omit issue comments
  -h, --help               Show this help

Dependencies: gh, jq

Examples:
  ./scripts/gh-issues-digest.sh
  ./scripts/gh-issues-digest.sh -o issues.md
  ./scripts/gh-issues-digest.sh --no-body --no-comments
  ./scripts/gh-issues-digest.sh -r rust-lang/rust -l 50
HELP
}

# --- Arg parsing ---
while [[ $# -gt 0 ]]; do
    case "$1" in
        -r|--repo)    REPO="$2"; shift 2 ;;
        -o|--output)  OUTPUT="$2"; shift 2 ;;
        -l|--limit)   LIMIT="$2"; shift 2 ;;
        --no-body)    INCLUDE_BODY=false; shift ;;
        --no-comments) INCLUDE_COMMENTS=false; shift ;;
        -h|--help)    usage; exit 0 ;;
        *)            echo "error: unknown option '$1'" >&2; usage >&2; exit 1 ;;
    esac
done

# --- Validate dependencies ---
for cmd in gh jq; do
    if ! command -v "$cmd" &>/dev/null; then
        echo "error: '$cmd' is required but not found" >&2
        exit 1
    fi
done

# --- Resolve repo ---
if [[ -z "$REPO" ]]; then
    REPO=$(gh repo view --json nameWithOwner --jq '.nameWithOwner')
fi
echo "Fetching open issues for $REPO (limit: $LIMIT)..." >&2

# --- Fetch issues ---
FIELDS="number,title,body,labels,assignees,milestone,createdAt,updatedAt,comments,author,url,reactionGroups,isPinned"
ISSUES_JSON=$(gh issue list \
    --repo "$REPO" \
    --state open \
    --limit "$LIMIT" \
    --json "$FIELDS")

ISSUE_COUNT=$(echo "$ISSUES_JSON" | jq 'length')
echo "Fetched $ISSUE_COUNT issues." >&2

if [[ "$ISSUE_COUNT" -eq 0 ]]; then
    echo "No open issues found for $REPO." >&2
    exit 0
fi

# --- Fetch linked pull requests via GraphQL (batched) ---
OWNER="${REPO%/*}"
REPO_NAME="${REPO#*/}"
BATCH_SIZE=50
GRAPHQL_MAP="{}"

issue_numbers=$(echo "$ISSUES_JSON" | jq -r '.[].number')
batch=()
batch_num=0

fetch_issue_details_batch() {
    local nums=("$@")
    local query="{ repository(owner: \"$OWNER\", name: \"$REPO_NAME\") {"
    for n in "${nums[@]}"; do
        query+=" i${n}: issue(number: $n) {"
        query+="   number"
        query+="   issueType { name }"
        query+="   timelineItems(itemTypes: [CROSS_REFERENCED_EVENT], first: 25) {"
        query+="     nodes { ... on CrossReferencedEvent { source { __typename"
        query+="       ... on PullRequest { number title state isDraft merged url }"
        query+="     } } }"
        query+="   }"
        query+=" }"
    done
    query+=" } }"

    local result
    result=$(gh api graphql -f query="$query" 2>/dev/null) || return 0

    # Extract PR data and issue type per issue
    echo "$result" | jq '
      .data.repository | to_entries | map(
        {
          key: (.value.number | tostring),
          value: {
            issueType: (.value.issueType.name // null),
            linkedPRs: [
              .value.timelineItems.nodes[]
              | select(.source.__typename == "PullRequest")
              | .source
              | {
                  number,
                  title,
                  url,
                  status: (
                    if .merged then "merged"
                    elif .state == "CLOSED" then "closed"
                    elif .isDraft then "open-draft"
                    else "open-review"
                    end
                  )
                }
            ]
          }
        }
      ) | from_entries
    '
}

# --- Fetch repository issue types and labels via GraphQL ---
echo "Fetching repository issue types and labels..." >&2
ISSUE_TYPES_JSON=$(gh api graphql -f query='
{
  repository(owner: "'"$OWNER"'", name: "'"$REPO_NAME"'") {
    issueTypes(first: 20) {
      nodes { name description }
    }
  }
}' | jq '.data.repository.issueTypes.nodes')

type_count=$(echo "$ISSUE_TYPES_JSON" | jq 'length')
echo "Found $type_count issue types." >&2
LABELS_JSON=$(gh api graphql --paginate -f query='
{
  repository(owner: "'"$OWNER"'", name: "'"$REPO_NAME"'") {
    labels(first: 100) {
      nodes {
        name
        description
        issuesOpen: issues(states: [OPEN]) { totalCount }
        issuesClosed: issues(states: [CLOSED]) { totalCount }
        prsOpen: pullRequests(states: [OPEN]) { totalCount }
        prsMerged: pullRequests(states: [MERGED]) { totalCount }
        prsClosed: pullRequests(states: [CLOSED]) { totalCount }
      }
    }
  }
}' | jq -s '[.[].data.repository.labels.nodes[]] | sort_by(.name | ascii_downcase)')

label_count=$(echo "$LABELS_JSON" | jq 'length')
echo "Found $label_count labels." >&2

echo "Fetching linked pull requests and issue types..." >&2
for n in $issue_numbers; do
    batch+=("$n")
    if [[ ${#batch[@]} -ge $BATCH_SIZE ]]; then
        batch_num=$((batch_num + 1))
        batch_result=$(fetch_issue_details_batch "${batch[@]}")
        if [[ -n "$batch_result" ]]; then
            GRAPHQL_MAP=$(echo "$GRAPHQL_MAP" "$batch_result" | jq -s '.[0] * .[1]')
        fi
        batch=()
    fi
done
# Final partial batch
if [[ ${#batch[@]} -gt 0 ]]; then
    batch_result=$(fetch_issue_details_batch "${batch[@]}")
    if [[ -n "$batch_result" ]]; then
        GRAPHQL_MAP=$(echo "$GRAPHQL_MAP" "$batch_result" | jq -s '.[0] * .[1]')
    fi
fi

pr_total=$(echo "$GRAPHQL_MAP" | jq '[.[].linkedPRs | length] | add // 0')
echo "Found $pr_total linked pull requests." >&2

# Merge GraphQL data (linked PRs + issue type) into issues JSON
ISSUES_JSON=$(echo "$ISSUES_JSON" "$GRAPHQL_MAP" | jq -s '
  .[0] as $issues | .[1] as $details |
  [$issues[] |
    ($details[.number | tostring] // {}) as $d |
    . + {linkedPRs: ($d.linkedPRs // []), issueType: ($d.issueType // null)}
  ]
')

# --- Output setup ---
if [[ -n "$OUTPUT" ]]; then
    TMPFILE=$(mktemp)
    trap 'rm -f "$TMPFILE"' EXIT
    exec > "$TMPFILE"
fi

TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")

# --- Section 1: Header and AI instructions ---
cat <<EOF
# GitHub Issues Digest: $REPO

> **Generated:** $TIMESTAMP | **State:** open | **Count:** $ISSUE_COUNT

## Instructions for AI Agents

This document contains all open issues for **$REPO**, ordered by creation
date (oldest first). Use it for triage, categorization, sizing, and
prioritization.

### Categorization Guidance

- **Labels** are the primary signal for category
- Issues **without labels** need categorization
- An issue may belong to multiple categories

### Sizing Rubric

| Size | Description |
|------|-------------|
| XS | Typo, config change, one-line fix |
| S | Single-file change, clear scope |
| M | Multi-file change, moderate complexity |
| L | Cross-crate change, new feature, requires design |
| XL | Architecture change, multi-PR effort |

### Prioritization Signals (strongest first)

1. Label: \`bug\` (correctness) > \`enhancement\` > \`techdebt\` > \`documentation\`
2. Assignee present — someone owns it
3. Milestone set — planned for a release
4. Reaction count — community interest
5. Comment count — active discussion
6. Age — older unresolved may be stale or blocked

EOF

# --- Section 2: Summary statistics ---
echo "$ISSUES_JSON" | jq -r --arg repo "$REPO" '
  sort_by(.createdAt) as $sorted |
  ($sorted | length) as $total |
  [$sorted[] | select(.labels | length > 0)] | length as $labeled |
  ($total - $labeled) as $unlabeled |
  [$sorted[] | select(.assignees | length > 0)] | length as $assigned |
  [$sorted[] | select(.milestone != null)] | length as $milestoned |
  ($sorted | first) as $oldest |
  ($sorted | last) as $newest |

  "## Summary\n",
  "| Metric | Value |",
  "|--------|-------|",
  "| Total issues | \($total) |",
  "| With labels | \($labeled) |",
  "| Unlabeled | \($unlabeled) |",
  "| With assignee | \($assigned) |",
  "| With milestone | \($milestoned) |",
  "| Oldest | \($oldest.createdAt[:10]) (#\($oldest.number)) |",
  "| Newest | \($newest.createdAt[:10]) (#\($newest.number)) |",
  "| With type | \([$sorted[] | select(.issueType != null)] | length) |",
  "| Without type | \([$sorted[] | select(.issueType == null)] | length) |",
  "| With linked PRs | \([$sorted[] | select(.linkedPRs | length > 0)] | length) |",
  ""
'

# Repository labels with descriptions and issue/PR counts
echo "$LABELS_JSON" | jq -r '
  if length == 0 then
    "### Repository Labels\n\nNo labels defined.\n"
  else
    "### Repository Labels\n",
    "| Label | Description | Issues (Open/Closed) | PRs (Open/Merged/Closed) |",
    "|-------|-------------|:--------------------:|:------------------------:|",
    (.[] |
      "| \(.name) | \(.description // "—") | \(.issuesOpen.totalCount)/\(.issuesClosed.totalCount) | \(.prsOpen.totalCount)/\(.prsMerged.totalCount)/\(.prsClosed.totalCount) |"
    ),
    ""
  end
'

# Issue types with descriptions and open counts (computed from fetched issues)
echo "$ISSUE_TYPES_JSON" "$ISSUES_JSON" | jq -r -s '
  .[0] as $types | .[1] as $issues |
  # Count open issues per type from our data
  ($issues | group_by(.issueType) | map({key: (.[0].issueType // ""), value: length}) | from_entries) as $counts |
  if ($types | length) == 0 then
    "### Issue Types\n\nNo issue types defined.\n"
  else
    "### Issue Types\n",
    "| Type | Description | Open Issues |",
    "|------|-------------|:-----------:|",
    ($types[] |
      "| \(.name) | \(.description // "—") | \($counts[.name] // 0) |"
    ),
    "| *(none)* | Issues without a type assigned | \($counts[""] // 0) |",
    ""
  end
'

# Assignee distribution
echo "$ISSUES_JSON" | jq -r '
  [.[].assignees[].login] | group_by(.) |
  map({name: .[0], count: length}) | sort_by(-.count) |
  if length == 0 then
    "### Assignee Distribution\n\nNo assignees found.\n"
  else
    "### Assignee Distribution\n",
    "| Assignee | Count |",
    "|----------|-------|",
    (.[] | "| \(.name) | \(.count) |"),
    ""
  end
'

# --- Section 3: Per-issue rendering ---
echo ""
echo "## Issues"
echo ""

echo "$ISSUES_JSON" | jq -r \
    --argjson include_body "$INCLUDE_BODY" \
    --argjson include_comments "$INCLUDE_COMMENTS" '
  sort_by(.createdAt) | .[] |

  # Format labels
  ([.labels[].name] | if length == 0 then "none" else join(", ") end) as $labels |

  # Format assignees
  ([.assignees[].login] | if length == 0 then "none" else join(", ") end) as $assignees |

  # Format milestone
  (.milestone.title // "none") as $milestone |

  # Format reactions
  ([.reactionGroups // [] | .[] | select(.users.totalCount > 0) |
    "\(.content): \(.users.totalCount)"] |
    if length == 0 then "0" else join(", ") end) as $reactions |

  # Comment count
  (.comments | length) as $comment_count |

  # Pinned marker
  (if .isPinned then " [PINNED]" else "" end) as $pinned |

  # --- Emit markdown ---
  "---\n" +
  "### #\(.number): \(.title)\($pinned)\n\n" +
  "- **URL:** \(.url)\n" +
  "- **Type:** \(.issueType // "none")\n" +
  "- **Author:** \(.author.login)\n" +
  "- **Created:** \(.createdAt[:10])\n" +
  "- **Updated:** \(.updatedAt[:10])\n" +
  "- **Labels:** \($labels)\n" +
  "- **Assignees:** \($assignees)\n" +
  "- **Milestone:** \($milestone)\n" +
  "- **Reactions:** \($reactions)\n" +
  "- **Comments:** \($comment_count)\n" +

  # Linked PRs
  (if (.linkedPRs | length) > 0 then
    "- **Linked PRs:** " +
    ([.linkedPRs[] | "#\(.number) (\(.status))"] | join(", ")) +
    "\n"
  else
    "- **Linked PRs:** none\n"
  end) +

  # Body
  (if $include_body then
    "\n#### Description\n\n" +
    (if (.body // "") == "" then
      "*No description provided.*\n"
    else
      (.body |
        if (. | length) > 10000 then
          .[:10000] + "\n\n*[truncated — see issue URL for full text]*\n"
        else
          . + "\n"
        end)
    end)
  else "" end) +

  # Comments
  (if $include_comments and $comment_count > 0 then
    "\n#### Comments (\($comment_count))\n\n" +
    ([.comments[] |
      "**\(.author.login)** (\(.createdAt[:10]), \(.authorAssociation)):\n\(.body)\n"
    ] | join("\n"))
  else "" end) +

  ""
'

# --- Finalize output ---
if [[ -n "$OUTPUT" ]]; then
    mv "$TMPFILE" "$OUTPUT"
    trap - EXIT
    echo "Written to $OUTPUT" >&2
fi
