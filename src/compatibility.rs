/// Cloudflare Speedtest release that defines cfbench's measurement baseline.
pub const SPEEDTEST_VERSION: &str = "v1.13.0";

/// Cloudflare Speedtest commit that defines cfbench's measurement baseline.
pub const SPEEDTEST_COMMIT: &str = "5954dee4cc83548a9e5031140df4548f71cd1458";

/// Version detail passed to Clap, which prefixes the executable name.
pub const VERSION_BANNER: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (Cloudflare Speedtest v1.13.0, 5954dee4cc83548a9e5031140df4548f71cd1458)"
);
