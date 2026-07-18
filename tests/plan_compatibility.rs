use std::time::Duration;

use cfbench::config::{IpMode, RunConfig};
use cfbench::plan::{MeasurementStep, default_cloudflare_plan};

#[test]
fn upstream_plan_matches_v1_11_0() {
    let plan = default_cloudflare_plan();

    assert_eq!(
        plan.upstream_commit,
        "cfc99a74fd8d5c2121d319aeb7894c6246202c65"
    );
    assert_eq!(plan.upstream_version, "v1.11.0");
    assert_eq!(
        plan.steps,
        &[
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Latency { packets: 20 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 9,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 1_000_000,
                count: 8,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 100_000,
                count: 8,
                bypass_finish: false,
            },
            MeasurementStep::PacketLossUnsupported {
                packets: 1_000,
                responses_wait_ms: 3_000,
            },
            MeasurementStep::Upload {
                bytes: 1_000_000,
                count: 6,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 10_000_000,
                count: 6,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 10_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 25_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 25_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 100_000_000,
                count: 3,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 50_000_000,
                count: 3,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 250_000_000,
                count: 2,
                bypass_finish: false,
            },
        ]
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
}

#[test]
fn filtering_disabled_directions_preserves_order_and_packet_loss_metadata() {
    let config = RunConfig {
        no_download: true,
        no_upload: true,
        ..RunConfig::default()
    };

    let filtered = default_cloudflare_plan().for_config(&config);

    assert_eq!(
        filtered.steps,
        &[
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Latency { packets: 20 },
            MeasurementStep::PacketLossUnsupported {
                packets: 1_000,
                responses_wait_ms: 3_000,
            },
        ]
    );
}

#[test]
fn disabling_download_preserves_upload_step_order() {
    let config = RunConfig {
        no_download: true,
        ..RunConfig::default()
    };

    let filtered = default_cloudflare_plan().for_config(&config);

    assert_eq!(
        filtered.steps,
        &[
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Latency { packets: 20 },
            MeasurementStep::Upload {
                bytes: 100_000,
                count: 8,
                bypass_finish: false,
            },
            MeasurementStep::PacketLossUnsupported {
                packets: 1_000,
                responses_wait_ms: 3_000,
            },
            MeasurementStep::Upload {
                bytes: 1_000_000,
                count: 6,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 10_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 25_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Upload {
                bytes: 50_000_000,
                count: 3,
                bypass_finish: false,
            },
        ]
    );
}

#[test]
fn disabling_upload_preserves_download_step_order() {
    let config = RunConfig {
        no_upload: true,
        ..RunConfig::default()
    };

    let filtered = default_cloudflare_plan().for_config(&config);

    assert_eq!(
        filtered.steps,
        &[
            MeasurementStep::Latency { packets: 1 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 1,
                bypass_finish: true,
            },
            MeasurementStep::Latency { packets: 20 },
            MeasurementStep::Download {
                bytes: 100_000,
                count: 9,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 1_000_000,
                count: 8,
                bypass_finish: false,
            },
            MeasurementStep::PacketLossUnsupported {
                packets: 1_000,
                responses_wait_ms: 3_000,
            },
            MeasurementStep::Download {
                bytes: 10_000_000,
                count: 6,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 25_000_000,
                count: 4,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 100_000_000,
                count: 3,
                bypass_finish: false,
            },
            MeasurementStep::Download {
                bytes: 250_000_000,
                count: 2,
                bypass_finish: false,
            },
        ]
    );
}
