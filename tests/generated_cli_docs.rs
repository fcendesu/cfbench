use std::fs;

use cfbench::cli::{CliCommand, CompletionShell};
use cfbench::cli_docs::write_command;

fn assert_generated_asset(path: &str, command: CliCommand) {
    let committed = fs::read(path).unwrap_or_else(|error| panic!("read {path}: {error}"));
    let mut generated = Vec::new();
    write_command(&command, &mut generated)
        .unwrap_or_else(|error| panic!("generate {path}: {error}"));

    assert_eq!(
        generated, committed,
        "{path} has drifted; regenerate it with the matching cfbench utility command"
    );
}

#[test]
fn committed_shell_completions_match_the_clap_command() {
    for (path, shell) in [
        ("assets/completions/cfbench.bash", CompletionShell::Bash),
        ("assets/completions/_cfbench", CompletionShell::Zsh),
        ("assets/completions/cfbench.fish", CompletionShell::Fish),
        (
            "assets/completions/_cfbench.ps1",
            CompletionShell::Powershell,
        ),
    ] {
        assert_generated_asset(path, CliCommand::Completions { shell });
    }
}

#[test]
fn committed_man_page_matches_the_clap_command() {
    assert_generated_asset("assets/man/cfbench.1", CliCommand::Man);
}
