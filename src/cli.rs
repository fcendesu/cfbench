use clap::{Parser, Subcommand, ValueEnum};

use crate::compatibility::VERSION_BANNER;

/// An unofficial native speed test using Cloudflare-compatible methodology.
///
/// Because native HTTP timing differs from browser timing, results are not
/// expected to be numerically identical to speed.cloudflare.com.
#[derive(Clone, Debug, Parser)]
#[command(
    version = VERSION_BANNER,
    about,
    long_about,
    args_conflicts_with_subcommands = true
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<CliCommand>,

    /// Use IPv4 only
    #[arg(long, conflicts_with = "ipv6")]
    pub ipv4: bool,

    /// Use IPv6 only
    #[arg(long, conflicts_with = "ipv4")]
    pub ipv6: bool,

    /// Emit versioned JSON to stdout
    #[arg(long)]
    pub json: bool,

    /// Skip download measurements
    #[arg(long)]
    pub no_download: bool,

    /// Skip upload measurements
    #[arg(long)]
    pub no_upload: bool,

    /// Disable latency probes during transfers
    #[arg(long)]
    pub no_loaded_latency: bool,

    #[arg(
        long,
        help = "Skip the default public IP and network metadata request",
        long_help = "Skip the default public IP and network metadata request.\n\nMetadata collection is enabled by default and includes the public IP, ASN, and approximate location already visible to Cloudflare. --no-metadata skips the request entirely and omits those metadata fields from output."
    )]
    pub no_metadata: bool,

    /// Per-request timeout
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 30,
        value_parser = clap::value_parser!(u64).range(1..=300)
    )]
    pub timeout: u64,

    /// Suppress normal output; report status with the exit code
    #[arg(short, long, conflicts_with_all = ["json", "verbose"])]
    pub quiet: bool,

    /// Show per-request measurement progress
    #[arg(long)]
    pub verbose: bool,

    /// Perform an informational reachability probe to Cloudflare's RPKI-invalid route
    #[arg(long)]
    pub rpki_check: bool,
}

#[derive(Clone, Debug, Subcommand)]
pub enum CliCommand {
    /// Generate a shell completion script
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: CompletionShell,
    },
    /// Generate the cfbench(1) manual page
    Man,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum CompletionShell {
    Bash,
    Zsh,
    Fish,
    Powershell,
}
