use std::time::Duration;

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
