use std::io::{self, Write};

use clap::CommandFactory;
use clap_complete::{Shell, generate};

use crate::cli::{Cli, CliCommand, CompletionShell};

/// Generate an offline command-line support artifact.
pub fn write_command(command: &CliCommand, writer: &mut dyn Write) -> io::Result<()> {
    match command {
        CliCommand::Completions { shell } => {
            let mut command = Cli::command();
            generate(Shell::from(*shell), &mut command, "cfbench", writer);
            Ok(())
        }
        CliCommand::Man => clap_mangen::Man::new(Cli::command()).render(writer),
    }
}

impl From<CompletionShell> for Shell {
    fn from(shell: CompletionShell) -> Self {
        match shell {
            CompletionShell::Bash => Self::Bash,
            CompletionShell::Zsh => Self::Zsh,
            CompletionShell::Fish => Self::Fish,
            CompletionShell::Powershell => Self::PowerShell,
        }
    }
}
