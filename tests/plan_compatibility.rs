use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::plan::{Direction, MeasurementStep, default_cloudflare_plan};

#[test]
fn upstream_plan_matches_v1_12_1() {
    let plan = default_cloudflare_plan();

    assert_eq!(
        plan.upstream_commit,
        "567aeade7b6e1fbeea98edddb6031c5877678866"
    );
    assert_eq!(plan.upstream_version, "v1.12.1");
    assert_eq!(plan.steps, expected_v1_12_1_steps());
}

#[test]
fn v1_12_1_interleaves_two_packet_latency_steps_between_transfer_groups() {
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

    let expected: Vec<_> = expected_v1_12_1_steps()
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

    let expected: Vec<_> = expected_v1_12_1_steps()
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

    let expected: Vec<_> = expected_v1_12_1_steps()
        .into_iter()
        .filter(|step| step.direction() != Some(Direction::Upload))
        .collect();
    assert_eq!(filtered.steps, expected);
}

fn expected_v1_12_1_steps() -> Vec<MeasurementStep> {
    vec![
        MeasurementStep::Latency { packets: 2 },
        download(100_000, 1, true),
        MeasurementStep::Latency { packets: 20 },
        download(100_000, 9, false),
        MeasurementStep::Latency { packets: 2 },
        download(1_000_000, 8, false),
        MeasurementStep::Latency { packets: 2 },
        upload(100_000, 8),
        MeasurementStep::Latency { packets: 2 },
        MeasurementStep::PacketLossUnsupported {
            packets: 1_000,
            responses_wait_ms: 3_000,
        },
        upload(1_000_000, 6),
        MeasurementStep::Latency { packets: 2 },
        download(10_000_000, 6, false),
        MeasurementStep::Latency { packets: 2 },
        upload(10_000_000, 4),
        MeasurementStep::Latency { packets: 2 },
        download(25_000_000, 4, false),
        MeasurementStep::Latency { packets: 2 },
        upload(25_000_000, 4),
        MeasurementStep::Latency { packets: 2 },
        download(100_000_000, 3, false),
        MeasurementStep::Latency { packets: 2 },
        upload(50_000_000, 3),
        MeasurementStep::Latency { packets: 2 },
        download(250_000_000, 2, false),
    ]
}

fn download(bytes: u64, count: u32, bypass_finish: bool) -> MeasurementStep {
    MeasurementStep::Download {
        bytes,
        count,
        bypass_finish,
    }
}

fn upload(bytes: u64, count: u32) -> MeasurementStep {
    MeasurementStep::Upload {
        bytes,
        count,
        bypass_finish: false,
    }
}
