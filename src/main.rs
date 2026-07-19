use std::io::{self, Write};
use std::process::ExitCode;

use cfbench::app::{
    AppError, OutputOptions, run_with_signal_and_progress, write_outcome, write_progress,
};
use cfbench::cli::Cli;
use cfbench::config::RunConfig;
use cfbench::error::TransportError;
use cfbench::plan::default_cloudflare_plan;
use cfbench::results::RunResult;
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
    let (outcome, progress_error) = match ReqwestTransport::new(config.clone()) {
        Ok(transport) => {
            let plan = default_cloudflare_plan().for_config(&config);
            let runner =
                Runner::new(transport, plan).with_loaded_latency(!config.no_loaded_latency);
            let run = run_with_signal_and_progress(
                &runner,
                tokio::signal::ctrl_c(),
                options,
                io::stderr(),
            )
            .await;
            (run.outcome, run.progress_error)
        }
        Err(error) => {
            let progress_error = write_progress(options, &mut io::stderr().lock()).err();
            (failed_outcome(error), progress_error)
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

fn failed_outcome(source: TransportError) -> RunOutcome {
    let mut result = RunResult::empty();
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
