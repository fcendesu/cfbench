use std::time::Duration;

use crate::cli::Cli;
use crate::error::ConfigError;

/// Address-family policy for a measurement run.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum IpMode {
    /// Let the resolver and connector choose the address family.
    #[default]
    Auto,
    /// Permit IPv4 connections only.
    V4Only,
    /// Permit IPv6 connections only.
    V6Only,
}

/// Validated settings used to prepare and execute a measurement run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunConfig {
    pub ip_mode: IpMode,
    pub request_timeout: Duration,
    pub no_download: bool,
    pub no_upload: bool,
    pub no_loaded_latency: bool,
}

impl Default for RunConfig {
    fn default() -> Self {
        Self {
            ip_mode: IpMode::Auto,
            request_timeout: Duration::from_secs(30),
            no_download: false,
            no_upload: false,
            no_loaded_latency: false,
        }
    }
}

impl TryFrom<Cli> for RunConfig {
    type Error = ConfigError;

    fn try_from(cli: Cli) -> Result<Self, Self::Error> {
        if cli.ipv4 && cli.ipv6 {
            return Err(ConfigError::ConflictingIpModes);
        }
        if !(1..=300).contains(&cli.timeout) {
            return Err(ConfigError::InvalidTimeout(cli.timeout));
        }

        let ip_mode = if cli.ipv4 {
            IpMode::V4Only
        } else if cli.ipv6 {
            IpMode::V6Only
        } else {
            IpMode::Auto
        };

        Ok(Self {
            ip_mode,
            request_timeout: Duration::from_secs(cli.timeout),
            no_download: cli.no_download,
            no_upload: cli.no_upload,
            no_loaded_latency: cli.no_loaded_latency,
        })
    }
}
