#!/usr/bin/env python3
"""Open one tracking issue when cfbench's Cloudflare baseline may be stale."""

import html
import json
import os
import pathlib
import re
import sys
import urllib.error
import urllib.parse
import urllib.request
from typing import NamedTuple


UPSTREAM_REPOSITORY = "cloudflare/speedtest"
ISSUE_MARKER = "<!-- cfbench-cloudflare-compatibility-monitor -->"
SENSITIVE_PATHS = {
    "src/config/defaultConfig.ts",
    "src/config/internalConfig.ts",
    "src/Results/MeasurementCalculations.ts",
    "src/Results/ScoresCalculations.ts",
    "src/types.ts",
    "src/utils/numbers.ts",
}
SENSITIVE_PREFIXES = (
    "src/engines/BandwidthEngine/",
    "src/engines/LoadNetworkEngine/",
    "src/engines/PacketLossEngine/",
)
SHA_PATTERN = re.compile(r"^[0-9a-f]{40}$")
TAG_PATTERN = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._+-]{0,127}$")
REPOSITORY_PATTERN = re.compile(r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$")


class MonitorError(RuntimeError):
    pass


class Baseline(NamedTuple):
    version: str
    commit: str


class RelevantChange(NamedTuple):
    sha: str
    paths: tuple


class Snapshot(NamedTuple):
    latest_version: str
    latest_commit: str
    head_commit: str
    relevant_changes: tuple


def require_string(value, field):
    if not isinstance(value, str) or not value:
        raise MonitorError(f"GitHub API response has invalid {field}")
    return value


def require_sha(value, field):
    sha = require_string(value, field)
    if not SHA_PATTERN.fullmatch(sha):
        raise MonitorError(f"GitHub API response has invalid {field}")
    return sha


def load_baseline(path):
    try:
        source = pathlib.Path(path).read_text(encoding="utf-8")
    except OSError as error:
        raise MonitorError(f"cannot read compatibility baseline: {error}") from error

    versions = re.findall(r'SPEEDTEST_VERSION:\s*&str\s*=\s*"([^"]+)"', source)
    commits = re.findall(r'SPEEDTEST_COMMIT:\s*&str\s*=\s*"([0-9a-f]+)"', source)
    if len(versions) != 1 or len(commits) != 1 or not SHA_PATTERN.fullmatch(commits[0]):
        raise MonitorError("compatibility baseline must contain one valid version and commit")
    return Baseline(versions[0], commits[0])


def is_sensitive_path(path):
    return path in SENSITIVE_PATHS or path.startswith(SENSITIVE_PREFIXES)


class GitHubClient:
    def __init__(self, token, api_url="https://api.github.com"):
        if not token:
            raise MonitorError("GITHUB_TOKEN is required")
        self.api_url = api_url.rstrip("/")
        self.headers = {
            "Accept": "application/vnd.github+json",
            "Authorization": f"Bearer {token}",
            "User-Agent": "cfbench-compatibility-monitor",
            "X-GitHub-Api-Version": "2022-11-28",
        }

    def request(self, method, path, payload=None):
        body = None
        headers = dict(self.headers)
        if payload is not None:
            body = json.dumps(payload).encode("utf-8")
            headers["Content-Type"] = "application/json"
        request = urllib.request.Request(
            self.api_url + path, data=body, headers=headers, method=method
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                raw = response.read()
        except urllib.error.HTTPError as error:
            raise MonitorError(
                f"GitHub API {method} {path} failed with HTTP {error.code}"
            ) from error
        except (urllib.error.URLError, TimeoutError) as error:
            raise MonitorError(f"GitHub API {method} {path} failed: {error}") from error
        try:
            return json.loads(raw)
        except (UnicodeDecodeError, json.JSONDecodeError) as error:
            raise MonitorError(f"GitHub API {method} {path} returned invalid JSON") from error

    def get(self, path):
        return self.request("GET", path)

    def post(self, path, payload):
        return self.request("POST", path, payload)


def collect_snapshot(client, baseline):
    release = client.get(f"/repos/{UPSTREAM_REPOSITORY}/releases/latest")
    if not isinstance(release, dict):
        raise MonitorError("latest release response must be an object")
    latest_version = require_string(release.get("tag_name"), "release tag")
    if not TAG_PATTERN.fullmatch(latest_version):
        raise MonitorError("GitHub API response has invalid release tag")
    encoded_tag = urllib.parse.quote(latest_version, safe="")
    release_commit = client.get(
        f"/repos/{UPSTREAM_REPOSITORY}/commits/{encoded_tag}"
    )
    if not isinstance(release_commit, dict):
        raise MonitorError("release commit response must be an object")
    latest_commit = require_sha(release_commit.get("sha"), "release commit")

    repository = client.get(f"/repos/{UPSTREAM_REPOSITORY}")
    if not isinstance(repository, dict):
        raise MonitorError("repository response must be an object")
    default_branch = require_string(repository.get("default_branch"), "default branch")
    encoded_branch = urllib.parse.quote(default_branch, safe="")
    head = client.get(f"/repos/{UPSTREAM_REPOSITORY}/commits/{encoded_branch}")
    if not isinstance(head, dict):
        raise MonitorError("default branch response must be an object")
    head_commit = require_sha(head.get("sha"), "default branch commit")

    comparison = client.get(
        f"/repos/{UPSTREAM_REPOSITORY}/compare/{baseline.commit}...{head_commit}"
    )
    if not isinstance(comparison, dict):
        raise MonitorError("comparison response must be an object")
    status = comparison.get("status")
    commits = comparison.get("commits")
    total_commits = comparison.get("total_commits")
    if status not in ("identical", "ahead") or not isinstance(commits, list):
        raise MonitorError("pinned commit is not an ancestor of the upstream default branch")
    if not isinstance(total_commits, int) or total_commits != len(commits):
        raise MonitorError("upstream comparison is incomplete or malformed")

    relevant = []
    for entry in commits:
        if not isinstance(entry, dict):
            raise MonitorError("comparison commit must be an object")
        sha = require_sha(entry.get("sha"), "comparison commit")
        detail = client.get(f"/repos/{UPSTREAM_REPOSITORY}/commits/{sha}")
        if not isinstance(detail, dict) or not isinstance(detail.get("files"), list):
            raise MonitorError("commit detail response is malformed")
        paths = []
        for file_entry in detail["files"]:
            if not isinstance(file_entry, dict):
                raise MonitorError("commit file entry must be an object")
            path = require_string(file_entry.get("filename"), "commit filename")
            if is_sensitive_path(path):
                paths.append(path)
        if paths:
            relevant.append(RelevantChange(sha, tuple(sorted(set(paths)))))

    return Snapshot(latest_version, latest_commit, head_commit, tuple(relevant))


def review_required(baseline, snapshot):
    return (
        snapshot.latest_version != baseline.version
        or bool(snapshot.relevant_changes)
    )


def safe_code(value):
    printable = "".join(
        character if character >= " " and character != "\x7f" else "�"
        for character in value
    )
    return "`" + html.escape(printable, quote=True).replace("`", "&#96;") + "`"


def issue_body(baseline, snapshot):
    compare_url = (
        f"https://github.com/{UPSTREAM_REPOSITORY}/compare/"
        f"{baseline.commit}...{snapshot.head_commit}"
    )
    release_url = (
        f"https://github.com/{UPSTREAM_REPOSITORY}/releases/tag/"
        f"{urllib.parse.quote(snapshot.latest_version, safe='')}"
    )
    lines = [
        ISSUE_MARKER,
        "Cloudflare Speedtest may have moved beyond cfbench's pinned compatibility baseline.",
        "",
        "## Baseline",
        "",
        f"- Pinned version: {safe_code(baseline.version)}",
        f"- Pinned commit: [{safe_code(baseline.commit)}](https://github.com/{UPSTREAM_REPOSITORY}/commit/{baseline.commit})",
        f"- Latest release: [{safe_code(snapshot.latest_version)}]({release_url})",
        f"- Latest release commit: [{safe_code(snapshot.latest_commit)}](https://github.com/{UPSTREAM_REPOSITORY}/commit/{snapshot.latest_commit})",
        f"- Default-branch head: [{safe_code(snapshot.head_commit)}](https://github.com/{UPSTREAM_REPOSITORY}/commit/{snapshot.head_commit})",
        f"- [Compare pinned baseline with default branch]({compare_url})",
        "",
        "## Methodology-sensitive changes",
        "",
    ]
    if snapshot.relevant_changes:
        for change in snapshot.relevant_changes:
            lines.append(
                f"- [{safe_code(change.sha)}](https://github.com/{UPSTREAM_REPOSITORY}/commit/{change.sha})"
            )
            for path in change.paths:
                lines.append(f"  - {safe_code(path)}")
    else:
        lines.append("No methodology-sensitive post-baseline paths were detected. The release tag changed.")
    lines.extend(
        [
            "",
            "## Review checklist",
            "",
            "- [ ] Measurement schedule and request counts",
            "- [ ] Payload sizes, thresholds, and constants",
            "- [ ] Server-Timing and response parsing",
            "- [ ] Timing boundaries and duration formulas",
            "- [ ] Statistical reductions and result accumulation",
            "- [ ] Authorization and endpoint behavior",
            "- [ ] Pinned conformance fixtures and regression tests",
            "- [ ] Compatibility documentation and version metadata",
            "",
            "This issue is generated as an informational review trigger; it does not assert that cfbench is incompatible.",
        ]
    )
    return "\n".join(lines) + "\n"


def open_tracking_issue(client, repository):
    page = 1
    while True:
        suffix = "" if page == 1 else f"&page={page}"
        path = f"/repos/{repository}/issues?state=open&per_page=100{suffix}"
        issues = client.get(path)
        if not isinstance(issues, list):
            raise MonitorError("open issues response must be an array")
        for issue in issues:
            if not isinstance(issue, dict):
                raise MonitorError("open issue entry must be an object")
            body = issue.get("body") or ""
            if not isinstance(body, str):
                raise MonitorError("open issue body must be a string or null")
            if ISSUE_MARKER in body:
                return issue
        if len(issues) < 100:
            return None
        page += 1


def monitor(client, baseline, repository):
    if not REPOSITORY_PATTERN.fullmatch(repository):
        raise MonitorError("GITHUB_REPOSITORY must use owner/name format")
    snapshot = collect_snapshot(client, baseline)
    if not review_required(baseline, snapshot):
        return "current"

    existing = open_tracking_issue(client, repository)
    if existing is not None:
        return "existing"

    title = f"Review Cloudflare Speedtest compatibility for {snapshot.latest_version}"
    client.post(
        f"/repos/{repository}/issues",
        {
            "title": title,
            "body": issue_body(baseline, snapshot),
            "labels": ["enhancement", "github_actions"],
        },
    )
    return "created"


def main():
    root = pathlib.Path(__file__).resolve().parents[1]
    baseline = load_baseline(root / "src" / "compatibility.rs")
    repository = os.environ.get("GITHUB_REPOSITORY", "")
    client = GitHubClient(os.environ.get("GITHUB_TOKEN", ""))
    outcome = monitor(client, baseline, repository)
    messages = {
        "current": "Cloudflare compatibility baseline is current.",
        "existing": "Compatibility review is already tracked by an open issue.",
        "created": "Opened a Cloudflare compatibility tracking issue.",
    }
    print(messages[outcome])


if __name__ == "__main__":
    try:
        main()
    except MonitorError as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
