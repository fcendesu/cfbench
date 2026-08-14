use std::fs;

fn workflow(path: &str) -> String {
    let contents = fs::read_to_string(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    normalize_line_endings(contents)
}

fn normalize_line_endings(contents: String) -> String {
    contents.replace("\r\n", "\n")
}

#[test]
fn ci_uses_the_five_native_release_targets() {
    let workflow = workflow(".github/workflows/ci.yml");

    assert!(workflow.contains(
        "on:\n  push:\n    branches:\n      - main\n  pull_request:\n    branches:\n      - main"
    ));
    let required = [
        ("linux-x86_64", "ubuntu-22.04"),
        ("linux-aarch64", "ubuntu-24.04-arm"),
        ("macos-aarch64", "macos-15"),
        ("macos-x86_64", "macos-15-intel"),
        ("windows-x86_64", "windows-2025"),
    ];
    for (name, runner) in required {
        assert!(workflow.contains(name), "missing required job {name}");
        assert!(
            workflow.contains(runner),
            "missing required runner {runner}"
        );
    }
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

    for target in [
        "x86_64-unknown-linux-gnu",
        "aarch64-unknown-linux-gnu",
        "aarch64-apple-darwin",
        "x86_64-apple-darwin",
        "x86_64-pc-windows-msvc",
    ] {
        assert!(workflow.contains(target), "missing release target {target}");
    }
    assert!(workflow.contains("actions/upload-artifact@v4"));
    assert!(workflow.contains("actions/download-artifact@v4"));
    assert!(workflow.contains("scripts/assemble-release.sh"));
    assert!(workflow.contains("--version"));
    assert!(workflow.contains("needs: [build-linux-x86_64, build-standalone]"));
    assert!(workflow.contains("permissions:\n      contents: write"));
    assert_eq!(workflow.matches("gh release create").count(), 1);
}

#[test]
fn workflow_contracts_normalize_windows_line_endings() {
    assert_eq!(
        normalize_line_endings("on:\r\n  push:\r\n".to_owned()),
        "on:\n  push:\n"
    );
}
