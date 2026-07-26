# Cloudflare Speedtest compatibility

cfbench 0.1.0 is based on [Cloudflare Speedtest v1.12.1](https://github.com/cloudflare/speedtest/releases/tag/v1.12.1), commit [`567aeade7b6e1fbeea98edddb6031c5877678866`](https://github.com/cloudflare/speedtest/commit/567aeade7b6e1fbeea98edddb6031c5877678866).

It matches the public default order of latency, download, upload, and loaded-latency work, including the v1.12.1 two-request latency phases interleaved between transfer groups. It uses the upstream median latency, 90th-percentile bandwidth, 10 ms bandwidth-eligibility, 250 ms loaded-latency, 400 ms probe-throttle, and 1000 ms adaptive-stop rules.

The tool streams response bodies and upload data, disables automatic response decompression and retries, reuses one client per run, and records negotiated HTTP version where reqwest exposes it.

## Intentional native differences

Cloudflare's browser engine uses `PerformanceResourceTiming`. A native reqwest client cannot reproduce browser resource timing, exact on-wire header accounting, browser cache/service-worker behavior, or v1.12.1's TCP connection/server-time-delta calibration. cfbench therefore reports actual payload bytes and native request/stream durations; it does not claim numerical identity with the website.

When `Server-Timing` has no usable Cloudflare duration, cfbench follows v1.12.1 and uses a 0 ms fallback. Packet loss and AIM scores are outside 0.1.0: packet loss is not shown or approximated with ICMP.
