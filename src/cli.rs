use clap::Parser;

/// An unofficial native speed test using Cloudflare-compatible methodology.
///
/// Because native HTTP timing differs from browser timing, results are not
/// expected to be numerically identical to speed.cloudflare.com.
#[derive(Clone, Debug, Parser)]
#[command(version, about, long_about)]
pub struct Cli {
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

    /// Do not request or display public IP and network metadata
    #[arg(long)]
    pub no_metadata: bool,

    /// Per-request timeout
    #[arg(
        long,
        value_name = "SECONDS",
        default_value_t = 30,
        value_parser = clap::value_parser!(u64).range(1..=300)
    )]
    pub timeout: u64,

    /// Suppress progress lines
    #[arg(short, long)]
    pub quiet: bool,
}
