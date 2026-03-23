#!/usr/bin/env python3

import json
import math
import os
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


def read_data(relative_dir: str) -> dict | None:
    path = Path(
        os.environ["GITHUB_WORKSPACE"],
        os.getenv("BENCHMARK_REPORTS_DIR", "benchmark-reports-repository"),
        relative_dir,
        "data.js",
    )
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8")
    if not text.startswith(PREFIX):
        return None
    try:
        return json.loads(text[len(PREFIX) :])
    except json.JSONDecodeError:
        return None


def suite_entries(data: dict | None, suite: str) -> list[dict]:
    entries_root = (data or {}).get("entries")
    entries = entries_root.get(suite) if isinstance(entries_root, dict) else None
    return entries if isinstance(entries, list) else []


def commit_info(entry: dict | None) -> dict:
    commit = entry.get("commit") if isinstance(entry, dict) else None
    return commit if isinstance(commit, dict) else {}


def find_last(entries: list[dict], sha: str) -> tuple[dict, int] | tuple[None, None]:
    for index in range(len(entries) - 1, -1, -1):
        entry = entries[index]
        if isinstance(entry, dict) and commit_info(entry).get("id") == sha:
            return entry, index
    return None, None


def find_previous(entries: list[dict], start: int | None, excluded: set[str]) -> dict | None:
    if start is None:
        return None
    for index in range(start - 1, -1, -1):
        entry = entries[index]
        sha = commit_info(entry).get("id")
        if sha and sha not in excluded:
            return entry
    return None


def metrics(entry: dict | None) -> dict[str, tuple[float, str]]:
    result = {}
    for bench in (entry or {}).get("benches", []):
        if not isinstance(bench, dict):
            continue
        name, value = bench.get("name"), bench.get("value")
        if isinstance(name, str) and isinstance(value, (int, float)) and math.isfinite(value):
            result[name] = (value, bench.get("unit") if isinstance(bench.get("unit"), str) else "")
    return result


def format_value(value: float | None, unit: str) -> str:
    if not isinstance(value, (int, float)) or not math.isfinite(value):
        return "—"
    if value >= 1:
        return f"{value:.3f} {unit}"
    if value >= 0.01:
        return f"{value:.4f} {unit}"
    if value >= 0.0001:
        return f"{value:.6f} {unit}"
    return f"{value:.2e} {unit}"


def format_delta(current: float | None, baseline: float | None) -> str:
    if (
        not isinstance(current, (int, float))
        or not math.isfinite(current)
        or not isinstance(baseline, (int, float))
        or not math.isfinite(baseline)
        or baseline == 0
    ):
        return "—"
    percent = (current - baseline) / baseline * 100
    return f"{'🟢' if percent <= -1 else '🔴' if percent >= 1 else '🟡'} {percent:+.2f}%"


def metric_names(head_metrics: dict[str, tuple[float, str]]) -> list[str]:
    preferred = [name for name in LABELS if name in head_metrics][:5]
    if preferred:
        return preferred
    return [
        name
        for name in head_metrics
        if name.startswith("BenchmarkReplayLog - ") and not name.endswith("commits")
    ][:5]


def message(entry: dict | None) -> str:
    return str(commit_info(entry).get("message", "")).replace("\r", " ").replace("\n", " ")


def gh(method: str, path: str, data: dict | None = None) -> list[dict] | dict | None:
    req = urllib.request.Request(
        f"{os.getenv('GITHUB_API_URL', 'https://api.github.com')}{path}",
        data=None if data is None else json.dumps(data).encode(),
        headers={
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {os.environ['GITHUB_TOKEN']}",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        method=method,
    )
    with urllib.request.urlopen(req) as response:
        body = response.read()
    return json.loads(body) if body else None


def main() -> None:
    owner, repo = os.environ["GITHUB_REPOSITORY"].split("/", 1)
    repo_url = f"{os.getenv('GITHUB_SERVER_URL', 'https://github.com')}/{owner}/{repo}"
    pr_number = int(os.environ["PR_NUMBER"])
    base_sha, head_sha = os.environ["BASE_SHA"], os.environ["HEAD_SHA"]
    base_requested = os.getenv("BASE_BENCHMARK_REQUESTED") == "true"
    base_outcome = os.getenv("BASE_BENCHMARK_OUTCOME") or "skipped"

    trend_entries = suite_entries(read_data(os.environ["PR_DATA_DIR"]), "Replay 10K PR")
    compare_entries = suite_entries(read_data(os.environ["COMPARISON_DATA_DIR"]), "Replay 10K PR Compare")
    head_trend, head_index = find_last(trend_entries, head_sha)
    if head_trend is None and trend_entries:
        head_trend, head_index = trend_entries[-1], len(trend_entries) - 1
    head_compare, _ = find_last(compare_entries, head_sha)
    base_trend, _ = find_last(trend_entries, base_sha)
    base_compare, _ = find_last(compare_entries, base_sha)
    previous = find_previous(trend_entries, head_index, {head_sha, base_sha})

    head_entry = head_compare or head_trend
    base_entry = base_compare or base_trend
    head_metrics = metrics(head_entry)
    previous_metrics = metrics(previous)
    base_metrics = metrics(base_entry)
    rows = [
        "| {} | {} | {} | {} |".format(
            LABELS.get(name, name.removeprefix("BenchmarkReplayLog - ")),
            format_value(*head_metrics[name]),
            format_delta(head_metrics[name][0], previous_metrics.get(name, (None, ""))[0]),
            format_delta(head_metrics[name][0], base_metrics.get(name, (None, ""))[0]),
        )
        for name in metric_names(head_metrics)
    ]

    def link(sha: str) -> str:
        return f"[{sha[:8]}]({repo_url}/commit/{sha})"

    previous_sha = commit_info(previous).get("id")
    base_status = (
        "Base data reused from history."
        if not base_requested
        else "Base benchmark executed in this run."
        if base_outcome == "success"
        else f"Base benchmark did not succeed (`{base_outcome}`)."
    )
    body = "\n".join(
        [
            MARKER,
            "## Replay 10K Benchmark Summary",
            "",
            f"**Head:** {link(head_sha)}{f' - {message(head_entry)}' if head_entry else ''}",
            f"**Base:** {link(base_sha)}{f' - {message(base_entry)}' if base_entry else ''}",
            (
                f"**Previous PR commit:** {link(previous_sha)} - {message(previous)}"
                if previous_sha
                else "**Previous PR commit:** _Not available yet_"
            ),
            "",
            "_Lower is better for all timing metrics._",
            "",
            "| Metric | Head | vs Previous | vs Base |",
            "| --- | ---: | ---: | ---: |",
            *(rows or ["| No parsed timing metrics | — | — | — |"]),
            "",
            f"**Status:** {base_status}",
            "",
            f"- [PR trend graph]({os.environ['TREND_URL']})",
            f"- [Base vs head graph]({os.environ['COMPARISON_URL']})",
        ]
    )

    comments = gh("GET", f"/repos/{owner}/{repo}/issues/{pr_number}/comments?per_page=100") or []
    existing = next(
        (
            comment
            for comment in comments
            if comment.get("user", {}).get("login") == "github-actions[bot]" and MARKER in comment.get("body", "")
        ),
        None,
    )
    if existing:
        gh("PATCH", f"/repos/{owner}/{repo}/issues/comments/{existing['id']}", {"body": body})
        return
    gh("POST", f"/repos/{owner}/{repo}/issues/{pr_number}/comments", {"body": body})


if __name__ == "__main__":
    main()
