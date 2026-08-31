/// Cloudflare Speedtest release that defines cfbench's measurement baseline.
pub const SPEEDTEST_VERSION: &str = "v1.13.1";

/// Cloudflare Speedtest commit that defines cfbench's measurement baseline.
pub const SPEEDTEST_COMMIT: &str = "b387f42dfe2103f11f8b4c978cdea48abfcc03b3";

/// Version detail passed to Clap, which prefixes the executable name.
pub const VERSION_BANNER: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (Cloudflare Speedtest v1.13.1, b387f42dfe2103f11f8b4c978cdea48abfcc03b3)"
);
