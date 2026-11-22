#!/usr/bin/env python3
"""Allocate one Next alpha release commit. Invoked only by workflow_dispatch."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import tomllib
from pathlib import Path

BASELINE = "c17f32bbc"
VERSION = re.compile(r"^0\.2\.0-alpha\.([1-9][0-9]*)$")
TAG_PREFIX = "next-v"
BRANCH_PREFIX = "next-release/"


def run(*args: str) -> str:
    return subprocess.check_output(args, text=True).strip()


def next_version(releases: list[dict], reservations: dict[str, tuple[str, str]], source: str) -> tuple[str, str | None]:
    for version, (reserved_source, commit) in reservations.items():
        if reserved_source == source:
            return version, commit
    numbers = [
        int(match[1])
        for version in [*(item["tag_name"].removeprefix(TAG_PREFIX) for item in releases if item["tag_name"].startswith(TAG_PREFIX)), *reservations]
        if (match := VERSION.fullmatch(version))
    ]
    return f"0.2.0-alpha.{max(numbers, default=0) + 1}", None


def set_versions(root: Path, version: str) -> list[Path]:
    if not VERSION.fullmatch(version):
        raise ValueError("Not a Next alpha version")
    names = set()
    paths = []
    for path in sorted((root / "next").glob("**/Cargo.toml")):
        if "target" in path.relative_to(root).parts:
            continue
        text = path.read_text(encoding="utf-8")
        package = tomllib.loads(text).get("package", {})
        if package.get("version") != "0.2.0-dev":
            continue
        names.add(package["name"])
        text, count = re.subn(r'^version = "0\.2\.0-dev"$', f'version = "{version}"', text, count=1, flags=re.M)
        if count != 1:
            raise ValueError(f"Missing package version in {path}")
        path.write_text(text, encoding="utf-8")
        paths.append(path)
    if "drifting" not in names:
        raise ValueError("The source does not contain the Next development application")
    lock = root / "next" / "Cargo.lock"
    parts = lock.read_text(encoding="utf-8").split("[[package]]")
    updated = set()
    for index in range(1, len(parts)):
        block = parts[index]
        name = re.search(r'^name = "([^"]+)"$', block, flags=re.M)
        if name and name[1] in names:
            parts[index], count = re.subn(r'^version = "0\.2\.0-dev"$', f'version = "{version}"', block, count=1, flags=re.M)
            if count != 1:
                raise ValueError(f"Lockfile version missing for {name[1]}")
            updated.add(name[1])
    if updated != names:
        raise ValueError("Release package set differs from the lockfile")
    lock.write_text("[[package]]".join(parts), encoding="utf-8")
    return [*paths, lock]


GROUPS = [
    ("模型兼容连接", ("provider", "gateway", "protocol", "model")),
    ("本地听写", ("dictation", "whisper")),
    ("外部 Agent", ("agent", "codex", "harness", "acp", "mcp")),
    ("背景与外观", ("background", "backdrop", "appearance")),
    ("桌面与导航", ("linux", "window", "navigation", "template edits")),
    ("番茄钟", ("pomodoro",)),
    ("时间线与历法", ("calendar", "timeline")),
    ("整书版本", ("whole-project", "whole-book", "version archives", "restores")),
    ("构建与发布", ("release", "ci gates", "ci/cd")),
]


def release_notes(version: str, source: str, commit: str, previous: str, logs: list[tuple[str, str]], repository: str) -> str:
    groups: dict[str, list[str]] = {}
    for sha, title in logs:
        group = next((name for name, words in GROUPS if any(word in title.lower() for word in words)), "其他改进")
        groups.setdefault(group, []).append(f"- {title} ({sha[:9]})")
    sections = [
        f"# Drifting Next {version}",
        f"源码：`{source}`\n\n发布提交：`{commit}`",
        f"比较范围：`{previous}..{source}`\n\nhttps://github.com/{repository}/compare/{previous}...{source}",
        "Linux x86_64 与 Windows x64 使用同一发布提交。安装包使用独立的 Next 身份，不替换 Tauri 的安装与更新通道。",
    ]
    for name, items in groups.items():
        sections.append("## " + name + "\n\n" + "\n".join(items))
    sections.append(f"<!-- drifting-next-source:{source} release-commit:{commit} -->")
    return "\n\n".join(sections) + "\n"


def prepare(source: str, output: Path) -> None:
    root = Path(run("git", "rev-parse", "--show-toplevel"))
    source = run("git", "rev-parse", f"{source}^{{commit}}")
    if run("git", "rev-parse", "HEAD") != source:
        raise ValueError("Prepare must start at the requested source commit")
    if run("git", "status", "--porcelain"):
        raise ValueError("Prepare requires an untouched source checkout")
    repository = os.environ["GITHUB_REPOSITORY"]
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("Invalid GitHub repository")
    pages = json.loads(run("gh", "api", "--paginate", "--slurp", f"repos/{repository}/releases?per_page=100"))
    releases = [item for page in pages for item in page]
    run("git", "fetch", "--no-tags", "origin", "+refs/heads/next-release/*:refs/remotes/origin/next-release/*")
    refs = run("git", "for-each-ref", "--format=%(refname)", "refs/remotes/origin/next-release/").splitlines()
    reservations = {}
    for ref in refs:
        version = ref.removeprefix("refs/remotes/origin/" + BRANCH_PREFIX)
        if not VERSION.fullmatch(version):
            continue
        reserved = run("git", "rev-parse", ref)
        trailer = re.search(r"^Next-Source: ([0-9a-f]{40})$", run("git", "show", "-s", "--format=%B", reserved), flags=re.M)
        if not trailer or run("git", "rev-parse", reserved + "^") != trailer[1]:
            raise ValueError(f"Invalid release reservation {version}")
        reservations[version] = (trailer[1], reserved)
    run("git", "merge-base", "--is-ancestor", BASELINE, source)
    version, commit = next_version(releases, reservations, source)
    tag = TAG_PREFIX + version
    published = any(item["tag_name"] == tag and not item["draft"] for item in releases)
    previous_releases = [
        item for item in releases
        if not item["draft"] and item["tag_name"].startswith(TAG_PREFIX)
        and VERSION.fullmatch(item["tag_name"].removeprefix(TAG_PREFIX))
        and item["tag_name"] != tag
    ]
    previous = max(previous_releases, key=lambda item: int(VERSION.fullmatch(item["tag_name"].removeprefix(TAG_PREFIX))[1]))["tag_name"] if previous_releases else BASELINE
    logs = [line.split("\t", 1) for line in run("git", "log", "--format=%H%x09%s", f"{previous}..{source}").splitlines()]
    if commit is None:
        paths = set_versions(root, version)
        run("git", "add", *(str(path.relative_to(root)) for path in paths))
        run("git", "-c", "user.name=github-actions[bot]", "-c", "user.email=41898282+github-actions[bot]@users.noreply.github.com", "commit", "-m", f"Release Drifting Next {version}", "-m", f"Next-Source: {source}")
        commit = run("git", "rev-parse", "HEAD")
        run("git", "push", "origin", f"HEAD:refs/heads/{BRANCH_PREFIX}{version}")
    output.mkdir(parents=True, exist_ok=True)
    (output / "notes.md").write_text(release_notes(version, source, commit, previous, logs, repository), encoding="utf-8")
    data = dict(version=version, tag=tag, commit=commit, source=source, published=str(published).lower())
    (output / "release.json").write_text(json.dumps(data, indent=2) + "\n", encoding="utf-8")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as file:
        for key, value in data.items():
            file.write(f"{key}={value}\n")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    prepare(args.source, args.output)
