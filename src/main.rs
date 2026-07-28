use std::io::{self, Write};
use std::process::ExitCode;

use cfbench::app::{
    AppError, OutputOptions, run_with_signal_and_progress, write_outcome, write_progress,
};
use cfbench::cli::Cli;
use cfbench::clock::RunClock;
use cfbench::config::RunConfig;
use cfbench::error::TransportError;
use cfbench::plan::default_cloudflare_plan;
use cfbench::results::{MetadataStatus, RunResult};
use cfbench::runner::{MeasurementTransport, RunOutcome, Runner};
use cfbench::transport::ReqwestTransport;
use clap::{CommandFactory, Parser, error::ErrorKind};

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let config = match RunConfig::try_from(cli.clone()) {
        Ok(config) => config,
        Err(error) => Cli::command()
            .error(ErrorKind::ValueValidation, error.to_string())
            .exit(),
    };

    run(cli, config).await
}

async fn run(cli: Cli, config: RunConfig) -> ExitCode {
    let options = OutputOptions {
        json: cli.json,
        quiet: cli.quiet,
        verbose: cli.verbose,
    };
    let (outcome, progress_error) = match prepare_runner(&config, ReqwestTransport::new) {
        Ok(runner) => {
            let run = run_with_signal_and_progress(
                &runner,
                tokio::signal::ctrl_c(),
                options,
                io::stderr(),
            )
            .await;
            (run.outcome, run.progress_error)
        }
        Err(outcome) => {
            let progress_error = write_progress(options, &mut io::stderr().lock()).err();
            (*outcome, progress_error)
        }
    };

    let output_status = exit_code_from_output_status(write_outcome(
        outcome,
        options,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ));

    if let Some(error) = progress_error {
        report_app_error(&error);
        ExitCode::FAILURE
    } else {
        output_status
    }
}

fn exit_code_from_output_status(status: Result<u8, AppError>) -> ExitCode {
    match status {
        Ok(status) => ExitCode::from(status),
        Err(error) => {
            report_app_error(&error);
            ExitCode::FAILURE
        }
    }
}

fn report_app_error(error: &AppError) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "error: {error}");
    let _ = stderr.flush();
}

fn prepare_runner<T>(
    config: &RunConfig,
    build_transport: impl FnOnce(RunConfig) -> Result<T, TransportError>,
) -> Result<Runner<T>, Box<RunOutcome>>
where
    T: MeasurementTransport,
{
    let clock = RunClock::start();
    let mut setup_result = RunResult::empty();
    setup_result.started_at = clock.started_at().to_owned();
    if config.no_metadata {
        setup_result.target.metadata_status = MetadataStatus::Disabled;
        setup_result.target.metadata = None;
    }
    let transport = build_transport(config.clone())
        .map_err(|error| Box::new(failed_outcome(error, setup_result)))?;
    let plan = default_cloudflare_plan().for_config(config);
    Ok(Runner::new(transport, plan)
        .with_loaded_latency(!config.no_loaded_latency)
        .with_metadata(!config.no_metadata)
        .with_rpki_check(config.rpki_check))
}

fn failed_outcome(source: TransportError, mut result: RunResult) -> RunOutcome {
    let error = cfbench::runner::RunnerError::Transport {
        stage: "setup".to_owned(),
        source,
    };
    result.failures.push(error.to_string());
    RunOutcome {
        result,
        error: Some(error),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use cfbench::app::EXIT_PARTIAL;
    use cfbench::cancellation::CancellationToken;
    use cfbench::output::render_text;
    use cfbench::plan::Direction;
    use cfbench::results::{
        LatencyPoint, MetadataStatus, RpkiReachability, RpkiReachabilityStatus,
    };
    use cfbench::runner::{MeasurementFuture, MeasurementTransport, RpkiFuture, RunnerError};

    use super::*;

    struct RpkiFlagProbeTransport {
        rpki_calls: Arc<AtomicUsize>,
    }

    impl MeasurementTransport for RpkiFlagProbeTransport {
        fn rpki_reachability<'a>(&'a self, _: &'a CancellationToken) -> RpkiFuture<'a> {
            Box::pin(async move {
                self.rpki_calls.fetch_add(1, Ordering::SeqCst);
                Ok(RpkiReachability {
                    status: RpkiReachabilityStatus::Reachable,
                    host: Some("invalid.rpki.cloudflare.com".to_owned()),
                    detail: None,
                })
            })
        }

        fn latency<'a>(&'a self, _: &'a CancellationToken) -> MeasurementFuture<'a> {
            Box::pin(async {
                Err(TransportError::HttpStatus {
                    endpoint: "https://fixture.invalid/__down".to_owned(),
                    status: 503,
                    payload_bytes: 0,
                })
            })
        }

        fn loaded_latency<'a>(
            &'a self,
            _: Direction,
            _: &'a CancellationToken,
        ) -> MeasurementFuture<'a> {
            unreachable!("loaded latency is disabled")
        }

        fn download<'a>(
            &'a self,
            _: u64,
            _: Option<&'a str>,
            _: &'a CancellationToken,
        ) -> MeasurementFuture<'a> {
            unreachable!("download is disabled")
        }

        fn upload<'a>(&'a self, _: u64, _: &'a CancellationToken) -> MeasurementFuture<'a> {
            unreachable!("upload is disabled")
        }
    }

    #[tokio::test]
    async fn prepare_runner_enables_rpki_check_from_run_config() {
        let rpki_calls = Arc::new(AtomicUsize::new(0));
        let config = RunConfig {
            no_download: true,
            no_upload: true,
            no_loaded_latency: true,
            no_metadata: true,
            rpki_check: true,
            ..RunConfig::default()
        };
        let runner = prepare_runner(&config, |_| {
            Ok(RpkiFlagProbeTransport {
                rpki_calls: rpki_calls.clone(),
            })
        })
        .expect("build scripted runner");

        let outcome = runner.run(&CancellationToken::new()).await;

        assert!(matches!(
            outcome.error,
            Some(RunnerError::Transport { ref stage, .. }) if stage == "latency"
        ));
        assert_eq!(rpki_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            outcome.result.rpki.status,
            RpkiReachabilityStatus::Reachable
        );
    }

    #[test]
    fn usable_partial_outcome_exits_two_at_the_process_boundary() {
        let mut result = RunResult::empty();
        result.raw.latency.push(LatencyPoint {
            ping_ms: 10.0,
            ttfb_ms: 20.0,
            server_time_ms: 0.0,
            http_version: Some("HTTP/2".to_owned()),
            measured_at_unix_ms: 0,
        });
        let outcome = RunOutcome {
            result,
            error: Some(RunnerError::Transport {
                stage: "download".to_owned(),
                source: TransportError::HttpStatus {
                    endpoint: "https://fixture.invalid/__down".to_owned(),
                    status: 503,
                    payload_bytes: 0,
                },
            }),
        };
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let status = write_outcome(
            outcome,
            OutputOptions {
                json: true,
                quiet: true,
                verbose: false,
            },
            &mut stdout,
            &mut stderr,
        )
        .unwrap();

        assert_eq!(status, EXIT_PARTIAL);
        assert_eq!(
            exit_code_from_output_status(Ok(status)),
            ExitCode::from(EXIT_PARTIAL)
        );
        serde_json::from_slice::<serde_json::Value>(&stdout).unwrap();
        assert!(
            String::from_utf8(stderr)
                .unwrap()
                .contains("error: transport failed")
        );
    }

    #[test]
    fn setup_failure_uses_prebuild_rfc3339_run_timestamp() {
        let outcome = match prepare_runner(&RunConfig::default(), |_| {
            Err::<ReqwestTransport, _>(TransportError::InvalidBaseUrl(
                "scripted setup failure".to_owned(),
            ))
        }) {
            Ok(_) => panic!("scripted transport construction must fail"),
            Err(outcome) => *outcome,
        };

        assert!(humantime::parse_rfc3339(&outcome.result.started_at).is_ok());
        assert_eq!(
            outcome.result.target.metadata_status,
            MetadataStatus::Unavailable
        );
        assert!(outcome.result.target.metadata.is_none());
    }

    #[test]
    fn setup_failure_applies_no_metadata_policy_before_transport_build() {
        let config = RunConfig {
            no_metadata: true,
            ..RunConfig::default()
        };
        let outcome = match prepare_runner(&config, |received_config| {
            assert!(received_config.no_metadata);
            Err::<ReqwestTransport, _>(TransportError::InvalidBaseUrl(
                "scripted setup failure".to_owned(),
            ))
        }) {
            Ok(_) => panic!("scripted transport construction must fail"),
            Err(outcome) => *outcome,
        };

        assert!(humantime::parse_rfc3339(&outcome.result.started_at).is_ok());
        assert_eq!(
            outcome.result.target.metadata_status,
            MetadataStatus::Disabled
        );
        assert!(outcome.result.target.metadata.is_none());
        let rendered = render_text(&outcome.result);
        assert!(!rendered.contains("Edge (informational):"));
        assert!(!rendered.contains("Network:"));
        assert!(!rendered.contains("Public IP:"));
        assert!(!rendered.contains("Metadata:"));
    }
}
