#!/usr/bin/env python3
"""Normalize benchmark data and apply theme to reports."""

import json
import math
import os
import shutil
import subprocess
import urllib.request
from pathlib import Path

PREFIX = "window.BENCHMARK_DATA = "
WORKDIR = Path(os.getenv("BENCHMARK_REPORTS_DIR", "benchmark-reports-repository"))


def env(name, default=""):
    v = os.getenv(name, default)
    if not v:
        raise SystemExit(f"Missing required env: {name}")
    return v


def git(*args, cwd=None):
    subprocess.run(["git", *args], check=True, cwd=cwd)


def load_data(path):
    if not path.is_file():
        return None
    text = path.read_text(encoding="utf-8")
    return json.loads(text[len(PREFIX) :]) if text.startswith(PREFIX) else None


def normalize(path, subjects):
    data = load_data(path)
    if not data or not isinstance(data.get("entries"), dict):
        return
    changed = False
    for suites in data["entries"].values():
        if not isinstance(suites, list):
            continue
        for suite in suites:
            if not isinstance(suite, dict):
                continue
            commit = suite.get("commit") or {}
            if isinstance(commit, dict) and (subj := subjects.get(commit.get("id"))):
                if commit.get("message") != subj:
                    commit["message"] = subj
                    changed = True
            benches = suite.get("benches")
            if not isinstance(benches, list):
                continue
            filtered = []
            for b in benches:
                if not isinstance(b, dict):
                    filtered.append(b)
                    continue
                if b.get("name") == "BenchmarkReplayLog":
                    changed = True
                    continue
                unit = b.get("unit")
                new_unit = (
                    "s"
                    if unit == "ns"
                    else f"s/{unit[3:]}"
                    if isinstance(unit, str) and unit.startswith("ns/")
                    else "s/op"
                    if unit == "ns/op"
                    else None
                )
                if new_unit:
                    v = b.get("value")
                    if isinstance(v, (int, float)) and math.isfinite(v):
                        b["value"] = v / 1_000_000_000
                    b["unit"] = new_unit
                    if isinstance(n := b.get("name"), str):
                        b["name"] = (
                            n.replace(" - ns/op", " - s/op")
                            .replace(" - ns/", " - s/")
                            .replace(" - ns", " - s")
                        )
                    changed = True
                filtered.append(b)
            if len(filtered) != len(benches):
                suite["benches"] = filtered
    if changed:
        path.write_text(PREFIX + json.dumps(data, indent=2), encoding="utf-8")


def apply_theme(report_dir, css):
    idx = report_dir / "index.html"
    if not idx.is_file():
        return
    (report_dir / "theme.css").write_text(css, encoding="utf-8")
    html = idx.read_text(encoding="utf-8")
    if 'href="theme.css"' not in html and "</head>" in html:
        idx.write_text(
            html.replace(
                "</head>", '  <link rel="stylesheet" href="theme.css">\n</head>', 1
            ),
            encoding="utf-8",
        )


def fix_base_commit_ids(path, base_sha, base_subject, head_sha):
    """Fix commit IDs in comparison data where benchmark-action recorded the
    base benchmark under the HEAD SHA (it ignores the ref input in PR context)."""
    data = load_data(path)
    if not data or not base_sha or not head_sha or base_sha == head_sha:
        return
    changed = False
    for suites in (data.get("entries") or {}).values():
        if not isinstance(suites, list) or len(suites) < 2:
            continue
        # Sort by date to find the earliest entry — that's the base benchmark
        dated = [(s.get("date", 0), s) for s in suites if isinstance(s, dict)]
        dated.sort(key=lambda x: x[0])
        for _, suite in dated:
            commit = suite.get("commit") or {}
            if commit.get("id") == head_sha:
                commit["id"] = base_sha
                if base_subject:
                    commit["message"] = base_subject
                changed = True
                break  # only fix the first (oldest) entry per suite
    if changed:
        path.write_text(PREFIX + json.dumps(data, indent=2), encoding="utf-8")
        print(f"Fixed base commit ID in {path}")


def main():
    base_sha = os.getenv("BASE_SHA")
    head_sha = os.getenv("HEAD_SHA")
    base_subject = os.getenv("BASE_SUBJECT")
    head_subject = os.getenv("HEAD_SUBJECT")
    subjects = {
        sha: subj
        for sha, subj in (
            (base_sha, base_subject),
            (head_sha, head_subject),
        )
        if sha and subj
    }
    dirs = [
        d.strip()
        for d in os.getenv("BENCHMARK_DATA_DIRS", "").splitlines()
        if d.strip()
    ]
    cmp_dir = os.getenv("COMPARISON_DATA_DIR", "")

    shutil.rmtree(WORKDIR, ignore_errors=True)
    git(
        "clone",
        "--depth",
        "1",
        "-b",
        env("BENCHMARK_REPORTS_BRANCH"),
        f"https://x-access-token:{env('BENCHMARK_TOKEN')}@github.com/{env('BENCHMARK_REPORTS_REPOSITORY')}.git",
        str(WORKDIR),
    )

    with urllib.request.urlopen(env("BENCHMARK_THEME_CSS_URL")) as r:
        css = r.read().decode()

    # Fix base commit IDs in comparison data before normalizing
    if cmp_dir:
        fix_base_commit_ids(
            WORKDIR / cmp_dir / "data.js", base_sha, base_subject, head_sha
        )

    for d in dirs:
        rd = WORKDIR / d
        normalize(rd / "data.js", subjects)
        apply_theme(rd, css)

    git("config", "user.name", "github-actions[bot]", cwd=WORKDIR)
    git(
        "config",
        "user.email",
        "github-actions[bot]@users.noreply.github.com",
        cwd=WORKDIR,
    )
    git("add", "-A", ".", cwd=WORKDIR)

    if (
        subprocess.run(["git", "diff", "--cached", "--quiet"], cwd=WORKDIR).returncode
        == 0
    ):
        print("No benchmark report changes to commit.")
        return 0
    git("commit", "-m", env("BENCHMARK_COMMIT_MESSAGE"), cwd=WORKDIR)
    git("push", "origin", env("BENCHMARK_REPORTS_BRANCH"), cwd=WORKDIR)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
