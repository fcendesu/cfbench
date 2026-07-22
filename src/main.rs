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
use cfbench::runner::{RunOutcome, Runner};
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

    let output_status = match write_outcome(
        outcome,
        options,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            report_app_error(&error);
            ExitCode::FAILURE
        }
    };

    if let Some(error) = progress_error {
        report_app_error(&error);
        ExitCode::FAILURE
    } else {
        output_status
    }
}

fn report_app_error(error: &AppError) {
    let mut stderr = io::stderr().lock();
    let _ = writeln!(stderr, "error: {error}");
    let _ = stderr.flush();
}

fn prepare_runner(
    config: &RunConfig,
    build_transport: impl FnOnce(RunConfig) -> Result<ReqwestTransport, TransportError>,
) -> Result<Runner<ReqwestTransport>, Box<RunOutcome>> {
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
        .with_metadata(!config.no_metadata))
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
    use cfbench::output::render_text;
    use cfbench::results::MetadataStatus;

    use super::*;

    #[test]
    fn setup_failure_uses_prebuild_rfc3339_run_timestamp() {
        let outcome = match prepare_runner(&RunConfig::default(), |_| {
            Err(TransportError::InvalidBaseUrl(
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
            Err(TransportError::InvalidBaseUrl(
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
        assert!(!rendered.contains("Edge:"));
        assert!(!rendered.contains("Network:"));
        assert!(!rendered.contains("Public IP:"));
        assert!(!rendered.contains("Metadata:"));
    }
}
