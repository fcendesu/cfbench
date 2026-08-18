import importlib.util
import pathlib
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]
MODULE_PATH = ROOT / "scripts" / "check_cloudflare_compatibility.py"
SPEC = importlib.util.spec_from_file_location("compatibility_monitor", MODULE_PATH)
MONITOR = importlib.util.module_from_spec(SPEC)
assert SPEC.loader is not None
SPEC.loader.exec_module(MONITOR)


class FakeGitHub:
    def __init__(self, responses):
        self.responses = responses
        self.posts = []

    def get(self, path):
        response = self.responses.get(path)
        if isinstance(response, Exception):
            raise response
        if response is None:
            raise AssertionError(f"unexpected GET {path}")
        return response

    def post(self, path, payload):
        self.posts.append((path, payload))
        return {"html_url": "https://github.com/example/cfbench/issues/42"}


def upstream_responses(*, tag="v1.13.0", tag_commit="a" * 40, head=None, commits=None):
    head = head or tag_commit
    commits = commits or []
    responses = {
        "/repos/cloudflare/speedtest/releases/latest": {"tag_name": tag},
        f"/repos/cloudflare/speedtest/commits/{tag}": {"sha": tag_commit},
        "/repos/cloudflare/speedtest": {"default_branch": "main"},
        "/repos/cloudflare/speedtest/commits/main": {"sha": head},
        f"/repos/cloudflare/speedtest/compare/{'a' * 40}...{head}": {
            "status": "identical" if head == "a" * 40 else "ahead",
            "total_commits": len(commits),
            "commits": [{"sha": commit[0]} for commit in commits],
        },
    }
    for sha, paths in commits:
        responses[f"/repos/cloudflare/speedtest/commits/{sha}"] = {
            "sha": sha,
            "html_url": f"https://github.com/cloudflare/speedtest/commit/{sha}",
            "commit": {"message": "Synthetic upstream change\n\nUntrusted details"},
            "files": [{"filename": path} for path in paths],
        }
    return responses


class CompatibilityMonitorTests(unittest.TestCase):
    def setUp(self):
        self.baseline = MONITOR.Baseline("v1.13.0", "a" * 40)

    def test_current_baseline_does_not_query_or_mutate_issues(self):
        client = FakeGitHub(upstream_responses())

        outcome = MONITOR.monitor(client, self.baseline, "example/cfbench")

        self.assertEqual(outcome, "current")
        self.assertEqual(client.posts, [])

    def test_new_release_opens_tracking_issue(self):
        responses = upstream_responses(tag="v1.14.0", tag_commit="b" * 40)
        responses["/repos/example/cfbench/issues?state=open&per_page=100"] = []
        client = FakeGitHub(responses)

        outcome = MONITOR.monitor(client, self.baseline, "example/cfbench")

        self.assertEqual(outcome, "created")
        self.assertEqual(len(client.posts), 1)
        path, payload = client.posts[0]
        self.assertEqual(path, "/repos/example/cfbench/issues")
        self.assertIn(MONITOR.ISSUE_MARKER, payload["body"])
        self.assertIn("v1.13.0", payload["body"])
        self.assertIn("v1.14.0", payload["body"])
        self.assertEqual(payload["labels"], ["enhancement", "github_actions"])

    def test_sensitive_post_baseline_commit_opens_tracking_issue(self):
        sha = "c" * 40
        responses = upstream_responses(
            head=sha,
            commits=[(sha, ["src/Results/MeasurementCalculations.ts"])],
        )
        responses["/repos/example/cfbench/issues?state=open&per_page=100"] = []
        client = FakeGitHub(responses)

        outcome = MONITOR.monitor(client, self.baseline, "example/cfbench")

        self.assertEqual(outcome, "created")
        body = client.posts[0][1]["body"]
        self.assertIn(sha, body)
        self.assertIn("src/Results/MeasurementCalculations.ts", body)

    def test_irrelevant_post_baseline_commit_does_not_open_issue(self):
        sha = "d" * 40
        client = FakeGitHub(
            upstream_responses(head=sha, commits=[(sha, ["docs/README.md"])])
        )

        outcome = MONITOR.monitor(client, self.baseline, "example/cfbench")

        self.assertEqual(outcome, "current")
        self.assertEqual(client.posts, [])

    def test_existing_open_tracking_issue_prevents_duplicate(self):
        responses = upstream_responses(tag="v1.14.0", tag_commit="b" * 40)
        responses["/repos/example/cfbench/issues?state=open&per_page=100"] = [
            {
                "body": f"Still under review\n{MONITOR.ISSUE_MARKER}",
                "html_url": "https://github.com/example/cfbench/issues/41",
            }
        ]
        client = FakeGitHub(responses)

        outcome = MONITOR.monitor(client, self.baseline, "example/cfbench")

        self.assertEqual(outcome, "existing")
        self.assertEqual(client.posts, [])

    def test_api_failure_is_not_treated_as_current(self):
        responses = upstream_responses()
        responses["/repos/cloudflare/speedtest/releases/latest"] = MONITOR.MonitorError(
            "rate limit exceeded"
        )

        with self.assertRaisesRegex(MONITOR.MonitorError, "rate limit"):
            MONITOR.monitor(FakeGitHub(responses), self.baseline, "example/cfbench")

    def test_baseline_is_read_from_rust_source_of_truth(self):
        baseline = MONITOR.load_baseline(ROOT / "src" / "compatibility.rs")

        self.assertEqual(baseline.version, "v1.13.0")
        self.assertEqual(len(baseline.commit), 40)

    def test_issue_body_escapes_untrusted_path_text(self):
        snapshot = MONITOR.Snapshot(
            "v1.14.0",
            "b" * 40,
            "c" * 40,
            (MONITOR.RelevantChange("c" * 40, ("src/types.ts`\n<script>",)),),
        )

        body = MONITOR.issue_body(self.baseline, snapshot)

        self.assertNotIn("<script>", body)
        self.assertNotIn("`\n<script>", body)
        self.assertIn("&lt;script&gt;", body)


if __name__ == "__main__":
    unittest.main()
