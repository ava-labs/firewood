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
    text = path.read_text()
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
        path.write_text(PREFIX + json.dumps(data, indent=2))


def apply_theme(report_dir, css):
    idx = report_dir / "index.html"
    if not idx.is_file():
        return
    (report_dir / "theme.css").write_text(css)
    html = idx.read_text()
    if 'href="theme.css"' not in html and "</head>" in html:
        idx.write_text(
            html.replace(
                "</head>", '  <link rel="stylesheet" href="theme.css">\n</head>', 1
            )
        )


def main():
    subjects = {
        sha: subj
        for sha, subj in (
            (os.getenv("BASE_SHA"), os.getenv("BASE_SUBJECT")),
            (os.getenv("HEAD_SHA"), os.getenv("HEAD_SUBJECT")),
        )
        if sha and subj
    }
    dirs = [
        d.strip()
        for d in os.getenv("BENCHMARK_DATA_DIRS", "").splitlines()
        if d.strip()
    ]

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
