use cfbench::clock::RunClock;

#[test]
fn run_clock_exposes_rfc3339_start_and_nondecreasing_epoch_milliseconds() {
    let clock = RunClock::start();
    let first = clock.now_unix_ms();
    let second = clock.now_unix_ms();

    assert!(humantime::parse_rfc3339(clock.started_at()).is_ok());
    assert!(first > 0);
    assert!(first <= second);
}
