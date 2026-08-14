use std::fs;

fn workflow(path: &str) -> String {
    fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"))
}

#[test]
fn ci_filters_pushes_and_pull_requests_to_main() {
    let workflow = workflow(".github/workflows/ci.yml");

    assert!(workflow.contains(
        "on:\n  push:\n    branches:\n      - main\n  pull_request:\n    branches:\n      - main"
    ));
    for runner in ["ubuntu-latest", "macos-latest", "windows-latest"] {
        assert!(
            workflow.contains(runner),
            "missing required runner {runner}"
        );
    }
}

#[test]
fn releases_remain_tag_only() {
    let workflow = workflow(".github/workflows/release.yml");

    assert!(workflow.contains("on:\n  push:\n    tags:\n      - \"v*\""));
    assert!(!workflow.contains("pull_request:"));
}
