use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::plan::{Direction, MeasurementStep, default_cloudflare_plan};

mod support;

use support::upstream_v1_13_0::fixture;

#[test]
fn compatibility_document_uses_the_shared_upstream_baseline() {
    let document = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();
    assert!(document.contains(cfbench::compatibility::SPEEDTEST_VERSION));
    assert!(document.contains(cfbench::compatibility::SPEEDTEST_COMMIT));
}

#[test]
fn compatibility_document_explains_the_informational_rpki_check() {
    let document = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();
    assert!(document.contains("--rpki-check"));
    assert!(document.contains("informational"));
    assert!(document.contains("not proof"));
}

#[test]
fn installation_document_uses_the_current_release_version() {
    let document = std::fs::read_to_string("docs/INSTALLATION.md").unwrap();
    assert!(!document.contains("0.3.1"));
    assert!(document.contains("CFBENCH_VERSION=0.3.2"));
    assert!(document.contains("cfbench-v0.3.2-SHA256SUMS.txt"));
    assert!(document.contains("cfbench_0.3.2-1_amd64.deb"));
    assert!(document.contains("cfbench-0.3.2-1.x86_64.rpm"));
}

#[test]
fn compatibility_document_names_exact_and_native_rules_separately() {
    let document = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();

    assert!(document.contains("accumulates all 42"));
    assert!(document.contains("cfRequestDuration"));
    assert!(document.contains("cfSpeed"));
    assert!(document.contains("Upload duration"));
    assert!(document.contains("PerformanceResourceTiming"));
    assert!(document.contains("server-time-delta calibration"));
    assert!(!document.contains("numerically identical"));
}

#[test]
fn public_docs_define_silent_quiet_and_distinct_exit_codes() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    let compatibility = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();

    for document in [&readme, &compatibility] {
        let document = document.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(document.contains("`--quiet`"));
        assert!(document.contains("exit"));
        assert!(document.contains("`2` means invalid command-line usage"));
        assert!(document.contains("`3` means a usable partial measurement"));
    }
    assert!(readme.contains("fully silent"));
    assert!(compatibility.contains("invalid command-line"));
}

#[test]
fn public_docs_define_dynamic_terminal_progress_without_a_tui() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    let compatibility = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();

    for document in [&readme, &compatibility] {
        let normalized = document.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(normalized.contains("interactive terminal"));
        assert!(normalized.contains("`--verbose`"));
        assert!(normalized.contains("redirected"));
        assert!(normalized.contains("`--json`"));
        assert!(normalized.contains("`--quiet`"));
    }
    assert!(readme.contains("single-line"));
    assert!(!readme.to_ascii_lowercase().contains("full-screen tui"));
}

#[test]
fn public_docs_define_live_transfer_telemetry() {
    let readme = std::fs::read_to_string("README.md").unwrap();
    let compatibility = std::fs::read_to_string("docs/COMPATIBILITY.md").unwrap();
    let normalized_readme = readme.split_whitespace().collect::<Vec<_>>().join(" ");
    let normalized_compatibility = compatibility
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");

    assert!(normalized_readme.contains("Download 100 MB 1/3 · 642 Mbps · 63% · loaded 32.4 ms"));
    assert!(normalized_readme.contains("provisional recent-window display"));
    assert!(normalized_readme.contains("current request"));
    assert!(normalized_readme.contains("running latency/jitter"));
    assert!(normalized_readme.contains("latest direction-local loaded-latency probe"));
    assert!(
        normalized_compatibility.contains("latest direction-local loaded latency when available")
    );
    assert!(normalized_readme.contains("Final p90/median reductions remain authoritative"));
    assert!(normalized_readme.contains("transport-consumption feedback"));
    assert!(normalized_readme.contains("`--verbose` prints permanent per-request lines instead"));
    assert!(normalized_compatibility.contains("`--verbose` retains permanent per-request lines"));
    assert!(normalized_readme.contains(
        "For redirected default output and in `--json` and `--quiet` modes, dynamic progress is not emitted; their behavior is unchanged"
    ));
    assert!(normalized_compatibility.contains(
        "For redirected default output and in `--json` and `--quiet` modes, dynamic progress is not emitted and behavior is unchanged"
    ));

    for document in [&readme, &compatibility] {
        let normalized = document.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(normalized.contains("`--verbose`"));
        assert!(normalized.contains("`--json`"));
        assert!(normalized.contains("`--quiet`"));
        assert!(normalized.contains("redirected"));
    }
    assert!(compatibility.contains("measurement isolation"));
}

#[test]
fn upstream_plan_matches_v1_13_0() {
    let plan = default_cloudflare_plan();
    let fixture = fixture();

    assert_eq!(plan.upstream_commit, fixture.upstream_commit);
    assert_eq!(plan.upstream_version, fixture.upstream_version);
    assert_eq!(plan.steps, expected_v1_13_0_steps());
    assert_eq!(
        plan.steps
            .iter()
            .filter_map(|step| match step {
                MeasurementStep::Latency { packets } => Some(*packets as usize),
                _ => None,
            })
            .sum::<usize>(),
        fixture.expected_idle_latency_points,
    );
}

#[test]
fn v1_13_0_interleaves_two_packet_latency_steps_between_transfer_groups() {
    let plan = default_cloudflare_plan();
    let two_packet_latencies: Vec<_> = plan
        .steps
        .iter()
        .enumerate()
        .filter_map(|(index, step)| {
            (*step == MeasurementStep::Latency { packets: 2 }).then_some(index)
        })
        .collect();

    assert_eq!(
        two_packet_latencies,
        vec![0, 4, 6, 8, 11, 13, 15, 17, 19, 21, 23]
    );
}

#[test]
fn run_config_defaults_to_auto_and_thirty_second_timeout() {
    let config = RunConfig::default();

    assert_eq!(config.ip_mode, IpMode::Auto);
    assert_eq!(config.request_timeout, Duration::from_secs(30));
    assert!(!config.no_download);
    assert!(!config.no_upload);
    assert!(!config.no_loaded_latency);
    assert!(!config.no_metadata);
    assert!(!config.verbose);
    assert!(!config.rpki_check);
}

#[test]
fn filtering_disabled_directions_retains_every_non_transfer_step_in_order() {
    let filtered = default_cloudflare_plan().for_config(&RunConfig {
        no_download: true,
        no_upload: true,
        ..RunConfig::default()
    });

    let expected: Vec<_> = expected_v1_13_0_steps()
        .into_iter()
        .filter(|step| step.direction().is_none())
        .collect();
    assert_eq!(filtered.steps, expected);
}

#[test]
fn disabling_download_preserves_the_complete_upload_order() {
    let filtered = default_cloudflare_plan().for_config(&RunConfig {
        no_download: true,
        ..RunConfig::default()
    });

    let expected: Vec<_> = expected_v1_13_0_steps()
        .into_iter()
        .filter(|step| step.direction() != Some(Direction::Download))
        .collect();
    assert_eq!(filtered.steps, expected);
}

#[test]
fn disabling_upload_preserves_the_complete_download_order() {
    let filtered = default_cloudflare_plan().for_config(&RunConfig {
        no_upload: true,
        ..RunConfig::default()
    });

    let expected: Vec<_> = expected_v1_13_0_steps()
        .into_iter()
        .filter(|step| step.direction() != Some(Direction::Upload))
        .collect();
    assert_eq!(filtered.steps, expected);
}

#[test]
fn dependabot_updates_use_the_deterministic_weekly_schedule() {
    let config = include_str!("../.github/dependabot.yml").replace("\r\n", "\n");

    assert_dependabot_schedule(&config);
    assert_dependabot_schedule(&config.replace('\n', "\r\n"));
}

fn assert_dependabot_schedule(config: &str) {
    let normalized = config.replace("\r\n", "\n");
    let updater_blocks: Vec<_> = normalized
        .split("\n  - package-ecosystem: ")
        .skip(1)
        .collect();
    let expected_schedule = "    schedule:\n      interval: weekly\n      day: monday\n      time: \"06:00\"\n      timezone: Europe/Istanbul";

    assert_eq!(updater_blocks.len(), 2);
    for (block, ecosystem) in updater_blocks.iter().zip(["cargo", "github-actions"]) {
        assert!(block.starts_with(ecosystem));
        assert!(block.contains(expected_schedule));
    }
}

fn expected_v1_13_0_steps() -> Vec<MeasurementStep> {
    fixture()
        .schedule
        .iter()
        .copied()
        .map(MeasurementStep::from)
        .collect()
}
