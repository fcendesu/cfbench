use std::io;
use std::process::ExitCode;

use cfbench::app::{OutputOptions, run_with_signal, write_outcome, write_progress};
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
    if let Err(error) = write_progress(options, &mut io::stderr().lock()) {
        eprintln!("error: {error}");
        return ExitCode::FAILURE;
    }

    let outcome = match ReqwestTransport::new(config.clone()) {
        Ok(transport) => {
            let plan = default_cloudflare_plan().for_config(&config);
            let runner =
                Runner::new(transport, plan).with_loaded_latency(!config.no_loaded_latency);
            run_with_signal(&runner, tokio::signal::ctrl_c()).await
        }
        Err(error) => failed_outcome(error),
    };

    match write_outcome(
        outcome,
        options,
        &mut io::stdout().lock(),
        &mut io::stderr().lock(),
    ) {
        Ok(0) => ExitCode::SUCCESS,
        Ok(_) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
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
