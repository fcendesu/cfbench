/// Cloudflare Speedtest release that defines cfbench's measurement baseline.
pub const SPEEDTEST_VERSION: &str = "v1.12.1";

/// Cloudflare Speedtest commit that defines cfbench's measurement baseline.
pub const SPEEDTEST_COMMIT: &str = "567aeade7b6e1fbeea98edddb6031c5877678866";

/// Version text shown by the public command-line interface.
pub const VERSION_BANNER: &str = concat!(
    "cfbench ",
    env!("CARGO_PKG_VERSION"),
    " (Cloudflare Speedtest v1.12.1, 567aeade7b6e1fbeea98edddb6031c5877678866)"
);
