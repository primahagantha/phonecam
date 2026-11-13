#!/usr/bin/env python3
"""
PhoneCam Release Notes & Contributor Profile Generator
=====================================================
Generates structured Markdown release notes from Conventional Commits
with author GitHub avatars, categorized changelogs, and pull request links.
"""

import os
import re
import subprocess
import sys
from collections import defaultdict

# Known GitHub username mappings by commit author name or email
CONTRIBUTOR_MAP = {
    "primahagantha": "primahagantha",
    "prima hagantha": "primahagantha",
    "prima": "primahagantha",
    "regenadejester": "RegenadeJester",
    "regenade jester": "RegenadeJester",
    "hagantha": "primahagantha",
}

CATEGORY_MAP = [
    ("🚀 Features & Enhancements", ["feat"]),
    ("🐛 Bug Fixes & Stability", ["fix"]),
    ("⚡ Performance Optimizations", ["perf"]),
    ("🏗 Architecture & Core Refactoring", ["refactor"]),
    ("📱 Android Streaming & Kotlin Engine", ["android"]),
    ("📹 Virtual Camera & Drivers", ["vcam"]),
    ("📝 Documentation, Skills & Standards", ["docs"]),
    ("👷 CI/CD, Multi-Platform & Packaging", ["ci", "build"]),
    ("🧹 Maintenance & Dependencies", ["chore", "style", "test"]),
]

def run_git(cmd):
    try:
        result = subprocess.run(
            ["git"] + cmd,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            check=True
        )
        return result.stdout.strip()
    except Exception as e:
        return ""

def get_latest_tags():
    tags = run_git(["tag", "--sort=-creatordate"]).splitlines()
    tags = [t.strip() for t in tags if t.strip()]
    return tags

def get_commits_range(tag_from=None, tag_to="HEAD"):
    if tag_from:
        commit_range = f"{tag_from}..{tag_to}"
    else:
        commit_range = tag_to
    
    # Format: hash|author_name|author_email|subject
    output = run_git(["log", commit_range, "--pretty=format:%H|%an|%ae|%s"])
    commits = []
    for line in output.splitlines():
        parts = line.strip().split("|", 3)
        if len(parts) == 4:
            commits.append({
                "hash": parts[0],
                "author_name": parts[1],
                "author_email": parts[2],
                "subject": parts[3],
            })
    return commits

def resolve_github_user(name, email):
    key_name = name.lower().strip()
    key_email = email.lower().split("@")[0].strip()
    if key_name in CONTRIBUTOR_MAP:
        return CONTRIBUTOR_MAP[key_name]
    if key_email in CONTRIBUTOR_MAP:
        return CONTRIBUTOR_MAP[key_email]
    
    # Heuristic for GitHub noreply emails (e.g., 12345+username@users.noreply.github.com)
    if "noreply.github.com" in email:
        match = re.search(r'(?:\d+\+)?([^@]+)@users\.noreply\.github\.com', email)
        if match:
            return match.group(1)
            
    # Default fallback
    clean_name = re.sub(r'[^a-zA-Z0-9-]', '', name)
    return clean_name if clean_name else "contributor"

def categorize_commit(subject):
    # Conventional commit regex: type(scope): message
    match = re.match(r'^([a-zA-Z]+)(?:\(([^\)]+)\))?!?:\s*(.+)$', subject)
    if match:
        ctype = match.group(1).lower()
        scope = match.group(2)
        desc = match.group(3)
        return ctype, scope, desc
    return "other", None, subject

def generate_markdown(version, commits):
    categorized = defaultdict(list)
    contributors = {}

    for c in commits:
        gh_user = resolve_github_user(c["author_name"], c["author_email"])
        if gh_user not in contributors:
            contributors[gh_user] = {
                "name": c["author_name"],
                "username": gh_user,
                "avatar": f"https://github.com/{gh_user}.png?size=64",
                "count": 0,
            }
        contributors[gh_user]["count"] += 1

        ctype, scope, desc = categorize_commit(c["subject"])
        short_hash = c["hash"][:7]
        
        # Link PR if #123 is present in desc
        linked_desc = re.sub(r'#(\d+)', r'[#\1](https://github.com/primahagantha/phonecam/pull/\1)', desc)
        entry = f"- {f'**({scope})** ' if scope else ''}{linked_desc} ([`{short_hash}`](https://github.com/primahagantha/phonecam/commit/{c['hash']})) - @{gh_user}"
        categorized[ctype].append(entry)

    md = []
    md.append(f"## PhoneCam Release {version}\n")
    md.append("> Ultra-low latency, native Rust PC webcam system for Android with zero cloud dependencies.\n")
    
    raw_ver = version.lstrip('v')
    # Highlights / Summary table
    md.append("### 📦 Release Assets & Artifacts")
    md.append(f"- **Windows 11 / 10 Installer**: `PhoneCam-Setup-{raw_ver}.exe` (NSIS all-in-one installer including desktop app, virtual camera drivers, bundled ADB, and Android APK)")
    md.append(f"- **Windows 64-bit Portable**: `PhoneCam-v{raw_ver}-windows-x64.zip` (Zero-install standalone bundle with `run-portable.bat` and 1-click driver registration scripts)")
    md.append(f"- **Linux 64-bit Binary**: `PhoneCam-v{raw_ver}-linux-x64.tar.gz` (Native binary supporting Arch, Debian/Ubuntu, Fedora/DNF with `v4l2loopback`)")
    md.append("- **Android APKs**: Universal package (`PhoneCam.apk`) + Multi-ABI splits (`arm64-v8a`, `armeabi-v7a`, `x86_64`)\n")

    # Categories
    categorized_rendered = set()
    for cat_title, prefixes in CATEGORY_MAP:
        entries = []
        for p in prefixes:
            if p in categorized:
                entries.extend(categorized[p])
                categorized_rendered.add(p)
        if entries:
            md.append(f"### {cat_title}")
            md.extend(entries)
            md.append("")

    # Leftover commits
    other_entries = []
    for k, v in categorized.items():
        if k not in categorized_rendered and v:
            other_entries.extend(v)
    if other_entries:
        md.append("### 📋 Other Changes")
        md.extend(other_entries)
        md.append("")

    # Contributors section with avatars & profiles
    md.append("### 👥 Contributors")
    md.append("| Avatar | Contributor | Commits | Profile |")
    md.append("| :---: | :--- | :---: | :--- |")
    for u, data in sorted(contributors.items(), key=lambda x: x[1]["count"], reverse=True):
        avatar_img = f'<img src="{data["avatar"]}" width="40" height="40" style="border-radius:50%;" />'
        profile_link = f'[@{data["username"]}](https://github.com/{data["username"]})'
        md.append(f'| {avatar_img} | **{data["name"]}** | {data["count"]} | {profile_link} |')
    md.append("")

    md.append("---")
    md.append(f"**Full Changelog**: https://github.com/primahagantha/phonecam/commits/{version}")
    return "\n".join(md)

def main():
    target_version = sys.argv[1] if len(sys.argv) > 1 else "v1.0.0"
    output_path = sys.argv[2] if len(sys.argv) > 2 else "release-notes.md"
    
    tags = get_latest_tags()
    prev_tag = None
    if tags:
        for t in tags:
            if t != target_version:
                prev_tag = t
                break
                
    commits = get_commits_range(tag_from=prev_tag, tag_to="HEAD")
    if not commits:
        commits = get_commits_range(tag_from=None, tag_to="HEAD")

    notes = generate_markdown(target_version, commits)
    with open(output_path, "w", encoding="utf-8") as f:
        f.write(notes)
    print(f"Generated release notes for {target_version} ({len(commits)} commits) -> {output_path}")

if __name__ == "__main__":
    main()
