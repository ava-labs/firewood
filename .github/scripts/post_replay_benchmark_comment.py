#!/usr/bin/env python3
"""Post replay benchmark comparison comment on a PR."""

import json
import math
import os
import sys
import urllib.error
import urllib.request
from pathlib import Path

PREFIX = "window.BENCHMARK_DATA = "
MARKER = "<!-- replay-10k-performance-summary -->"
LABELS = {
    "BenchmarkReplayLog - s/op": "Total replay",
    "BenchmarkReplayLog - s/commit": "Commit",
    "BenchmarkReplayLog - s/propose-on-db": "Propose on DB",
    "BenchmarkReplayLog - s/propose-on-proposal": "Propose on proposal",
    "BenchmarkReplayLog - s/batch": "Batch update",
    "BenchmarkReplayLog - s/get-latest": "Get latest",
}


def load_data(rel_dir):
    p = Path(
        os.environ["GITHUB_WORKSPACE"],
        os.getenv("BENCHMARK_REPORTS_DIR", "benchmark-reports-repository"),
        rel_dir,
        "data.js",
    )
    print(f"  Loading: {p} (exists={p.is_file()})")
    if not p.is_file():
        return None
    text = p.read_text(encoding="utf-8")
    if not text.startswith(PREFIX):
        print(f"  WARNING: data.js missing expected prefix")
        return None
    return json.loads(text[len(PREFIX) :])


def entries(data, suite):
    return (data or {}).get("entries", {}).get(suite) or []


def find_by_sha(ents, sha):
    for i in range(len(ents) - 1, -1, -1):
        if isinstance(ents[i], dict) and ents[i].get("commit", {}).get("id") == sha:
            return ents[i], i
    return None, None


def get_metrics(entry):
    return {
        b["name"]: (b["value"], b.get("unit", ""))
        for b in (entry or {}).get("benches", [])
        if isinstance(b.get("value"), (int, float)) and math.isfinite(b["value"])
    }


def fmt_val(v, u):
    if not isinstance(v, (int, float)) or not math.isfinite(v):
        return "\u2014"
    for threshold, places in [(1, 3), (0.01, 4), (1e-4, 6)]:
        if v >= threshold:
            return f"{v:.{places}f} {u}"
    return f"{v:.2e} {u}"


def fmt_delta(cur, base):
    if (
        not all(isinstance(x, (int, float)) and math.isfinite(x) for x in (cur, base))
        or base == 0
    ):
        return "\u2014"
    p = (cur - base) / base * 100
    icon = "\U0001f7e2" if p <= -1 else "\U0001f534" if p >= 1 else "\U0001f7e1"
    return f"{icon} {p:+.2f}%"


def gh(method, path, data=None):
    url = f"{os.getenv('GITHUB_API_URL', 'https://api.github.com')}{path}"
    req = urllib.request.Request(
        url,
        data=json.dumps(data).encode() if data else None,
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        method=method,
    )
    try:
        with urllib.request.urlopen(req) as r:
            body = r.read()
            print(f"  API {method} {path} -> {r.status}")
            return json.loads(body or "null")
    except urllib.error.HTTPError as e:
        print(
            f"  API {method} {path} -> {e.code}: {e.read().decode(errors='replace')}",
            file=sys.stderr,
        )
        raise


def main():
    print("=== Post replay benchmark comment ===")
    owner, repo = os.environ["GITHUB_REPOSITORY"].split("/", 1)
    repo_url = f"{os.getenv('GITHUB_SERVER_URL', 'https://github.com')}/{owner}/{repo}"
    pr, base_sha, head_sha = (
        int(os.environ["PR_NUMBER"]),
        os.environ["BASE_SHA"],
        os.environ["HEAD_SHA"],
    )
    print(f"PR #{pr}, base={base_sha[:8]}, head={head_sha[:8]}")

    print("Loading trend data...")
    trend = entries(load_data(os.environ["PR_DATA_DIR"]), "Replay 10K PR")
    print(f"  Found {len(trend)} trend entries")
    print("Loading comparison data...")
    compare = entries(
        load_data(os.environ["COMPARISON_DATA_DIR"]), "Replay 10K PR Compare"
    )
    print(f"  Found {len(compare)} comparison entries")

    head_t, head_idx = find_by_sha(trend, head_sha)
    if head_t is None and trend:
        head_t, head_idx = trend[-1], len(trend) - 1
    head_c, _ = find_by_sha(compare, head_sha)
    base_t, _ = find_by_sha(trend, base_sha)
    base_c, _ = find_by_sha(compare, base_sha)
    print(
        f"  head_trend={'found' if head_t else 'missing'}, head_cmp={'found' if head_c else 'missing'}, base_trend={'found' if base_t else 'missing'}, base_cmp={'found' if base_c else 'missing'}"
    )

    prev = None
    if head_idx is not None:
        for i in range(head_idx - 1, -1, -1):
            s = trend[i].get("commit", {}).get("id")
            if s and s not in {head_sha, base_sha}:
                prev = trend[i]
                break

    hm, bm, pm = (
        get_metrics(head_c or head_t),
        get_metrics(base_c or base_t),
        get_metrics(prev),
    )
    print(
        f"  head metrics: {len(hm)}, base metrics: {len(bm)}, prev metrics: {len(pm)}"
    )
    names = [n for n in LABELS if n in hm][:5] or [
        n
        for n in hm
        if n.startswith("BenchmarkReplayLog - ") and not n.endswith("commits")
    ][:5]

    link = lambda s: f"[{s[:8]}]({repo_url}/commit/{s})"
    msg = lambda e: str((e or {}).get("commit", {}).get("message", "")).replace(
        "\n", " "
    )

    rows = (
        "\n".join(
            f"| {LABELS.get(n, n.removeprefix('BenchmarkReplayLog - '))} "
            f"| {fmt_val(*hm[n])} "
            f"| {fmt_delta(hm[n][0], pm.get(n, (None,))[0])} "
            f"| {fmt_delta(hm[n][0], bm.get(n, (None,))[0])} |"
            for n in names
        )
        or "| No parsed timing metrics | \u2014 | \u2014 | \u2014 |"
    )

    prev_sha = (prev or {}).get("commit", {}).get("id")
    base_req = os.getenv("BASE_BENCHMARK_REQUESTED") == "true"
    base_out = os.getenv("BASE_BENCHMARK_OUTCOME") or "skipped"
    status = (
        "Base benchmark executed."
        if base_req and base_out == "success"
        else f"Base benchmark: `{base_out}`."
        if base_req
        else "Base data reused from history."
    )

    he, be = head_c or head_t, base_c or base_t
    body = f"""{MARKER}
## Replay 10K Benchmark Summary

**Head:** {link(head_sha)}{f" - {msg(he)}" if he else ""}
**Base:** {link(base_sha)}{f" - {msg(be)}" if be else ""}
**Previous PR commit:** {f"{link(prev_sha)} - {msg(prev)}" if prev_sha else "_Not available yet_"}

_Lower is better for all timing metrics._

| Metric | Head | vs Previous | vs Base |
| --- | ---: | ---: | ---: |
{rows}

**Status:** {status}

- [PR trend graph]({os.environ["TREND_URL"]})
- [Base vs head graph]({os.environ["COMPARISON_URL"]})"""

    print(f"Posting new comment on PR #{pr}...")
    gh("POST", f"/repos/{owner}/{repo}/issues/{pr}/comments", {"body": body})
    print("Done.")


if __name__ == "__main__":
    main()
