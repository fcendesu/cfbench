use std::fs;

fn workflow(path: &str) -> String {
    let contents = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    normalize_line_endings(contents)
}

fn normalize_line_endings(contents: String) -> String {
    contents.replace("\r\n", "\n")
}

fn between<'a>(contents: &'a str, start: &str, end: &str) -> &'a str {
    contents
        .split_once(start)
        .unwrap_or_else(|| panic!("missing section start: {start}"))
        .1
        .split_once(end)
        .unwrap_or_else(|| panic!("missing section end: {end}"))
        .0
}

fn values<'a>(section: &'a str, prefix: &str) -> Vec<&'a str> {
    section
        .lines()
        .filter_map(|line| line.trim().strip_prefix(prefix))
        .collect()
}

#[test]
fn ci_uses_the_five_native_release_targets() {
    let workflow = workflow(".github/workflows/ci.yml");

    assert!(workflow.contains(
        "on:\n  push:\n    branches:\n      - main\n  pull_request:\n    branches:\n      - main"
    ));
    let matrix = between(
        &workflow,
        "      matrix:\n        include:\n",
        "\n\n    steps:",
    );
    assert_eq!(
        values(matrix, "- name: "),
        [
            "linux-x86_64",
            "linux-aarch64",
            "macos-aarch64",
            "macos-x86_64",
            "windows-x86_64",
        ]
    );
    assert_eq!(
        values(matrix, "os: "),
        [
            "ubuntu-22.04",
            "ubuntu-24.04-arm",
            "macos-15",
            "macos-15-intel",
            "windows-2025",
        ]
    );
    assert_eq!(
        values(matrix, "primary: "),
        ["true", "false", "false", "false", "false"]
    );
    assert_eq!(workflow.matches("key: ${{ matrix.name }}").count(), 1);
    assert!(!workflow.contains("gh release create"));
    assert!(!workflow.contains("contents: write"));
    assert!(!workflow.contains("tags:"));
}

#[test]
fn releases_remain_tag_only() {
    let workflow = workflow(".github/workflows/release.yml");

    assert!(workflow.contains("on:\n  push:\n    tags:\n      - \"v*\""));
    assert!(!workflow.contains("pull_request:"));
}

#[test]
fn release_builds_and_publishes_the_complete_native_matrix() {
    let workflow = workflow(".github/workflows/release.yml");

    let jobs = workflow.split_once("jobs:\n").unwrap().1;
    let job_ids: Vec<_> = jobs
        .lines()
        .filter_map(|line| {
            let candidate = line.strip_prefix("  ")?;
            (!candidate.starts_with(' '))
                .then(|| candidate.strip_suffix(':'))
                .flatten()
        })
        .collect();
    assert_eq!(
        job_ids,
        [
            "verify-source",
            "build-linux-x86_64",
            "build-standalone",
            "publish"
        ]
    );

    let matrix = between(
        &workflow,
        "      matrix:\n        include:\n",
        "\n\n    steps:",
    );
    assert_eq!(
        values(matrix, "target: "),
        [
            "aarch64-unknown-linux-gnu",
            "aarch64-apple-darwin",
            "x86_64-apple-darwin",
            "x86_64-pc-windows-msvc",
        ]
    );
    assert_eq!(workflow.matches("x86_64-unknown-linux-gnu").count(), 1);
    assert_eq!(workflow.matches("key: ${{ matrix.name }}").count(), 1);
    assert!(workflow.contains("actions/upload-artifact@v4"));
    assert!(workflow.contains("actions/download-artifact@v4"));
    assert!(workflow.contains("scripts/assemble-release.sh"));
    assert!(workflow.contains("--version"));
    assert!(workflow.contains("needs: [build-linux-x86_64, build-standalone]"));
    assert_eq!(
        workflow
            .matches("permissions:\n      contents: write")
            .count(),
        1
    );
    assert_eq!(workflow.matches("gh release create").count(), 1);
}

#[test]
fn workflow_contracts_normalize_windows_line_endings() {
    assert_eq!(
        normalize_line_endings("on:\r\n  push:\r\n".to_owned()),
        "on:\n  push:\n"
    );
}
