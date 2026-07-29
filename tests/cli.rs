use assert_cmd::Command;
use cfbench::cli::Cli;
use cfbench::config::RunConfig;
use clap::Parser;
use predicates::prelude::*;

#[test]
fn ip_family_flags_conflict() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .args(["--ipv4", "--ipv6"])
        .assert()
        .failure()
        .code(2);
}

#[test]
fn timeout_rejects_values_outside_one_through_three_hundred_seconds() {
    for seconds in ["0", "301"] {
        Command::cargo_bin("cfbench")
            .unwrap()
            .args(["--timeout", seconds])
            .assert()
            .failure()
            .code(2);
    }
}

#[test]
fn help_discloses_unofficial_native_methodology() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("unofficial"))
        .stdout(predicate::str::contains(
            "Cloudflare-compatible methodology",
        ))
        .stdout(predicate::str::contains(
            "native HTTP timing differs from browser timing",
        ));
}

#[test]
fn help_exposes_only_the_prd_flags() {
    let output = Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    for flag in [
        "--ipv4",
        "--ipv6",
        "--json",
        "--no-download",
        "--no-upload",
        "--no-loaded-latency",
        "--no-metadata",
        "--timeout",
        "--quiet",
        "--verbose",
        "--rpki-check",
        "--help",
        "--version",
    ] {
        assert!(stdout.contains(flag), "missing {flag} in help:\n{stdout}");
    }
    assert!(!stdout.contains("--base-url"));
    assert!(!stdout.contains("--provider"));
    assert!(stdout.contains("Skip the default public IP and network metadata request"));
}

#[test]
fn help_discloses_default_metadata_collection_and_request_opt_out() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Metadata collection is enabled by default",
        ))
        .stdout(predicate::str::contains(
            "public IP, ASN, and approximate location",
        ))
        .stdout(predicate::str::contains("already visible to Cloudflare"))
        .stdout(predicate::str::contains("--no-metadata skips the request"));
}

#[test]
fn no_metadata_is_public_and_defaults_to_collection() {
    let default = Cli::try_parse_from(["cfbench"]).unwrap();
    assert!(!RunConfig::try_from(default).unwrap().no_metadata);

    let disabled = Cli::try_parse_from(["cfbench", "--no-metadata"]).unwrap();
    assert!(RunConfig::try_from(disabled).unwrap().no_metadata);
}

#[test]
fn verbose_and_rpki_check_are_public_flags() {
    let cli = Cli::try_parse_from(["cfbench", "--verbose", "--rpki-check"]).unwrap();
    let config = RunConfig::try_from(cli).unwrap();

    assert!(config.verbose);
    assert!(config.rpki_check);

    for arguments in [
        ["cfbench", "--quiet", "--json"],
        ["cfbench", "--quiet", "--verbose"],
    ] {
        let error = Cli::try_parse_from(arguments).unwrap_err();
        assert_eq!(error.kind(), clap::error::ErrorKind::ArgumentConflict);
    }
}

#[test]
fn quiet_conflicts_with_json_and_verbose_before_runtime() {
    for conflicting in ["--json", "--verbose"] {
        Command::cargo_bin("cfbench")
            .unwrap()
            .args(["--quiet", conflicting])
            .assert()
            .failure()
            .code(2)
            .stdout(predicate::str::is_empty())
            .stderr(predicate::str::contains("--quiet").and(predicate::str::contains(conflicting)));
    }
}

#[test]
fn help_defines_quiet_as_exit_code_only() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Suppress normal output; report status with the exit code",
        ));
}

#[test]
fn help_describes_verbose_progress_and_informational_rpki_reachability() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains(
            "Show per-request measurement progress",
        ))
        .stdout(predicate::str::contains(
            "Perform an informational reachability probe to Cloudflare's RPKI-invalid route",
        ));
}

#[test]
fn version_exactly_identifies_cfbench_and_the_compatibility_baseline() {
    Command::cargo_bin("cfbench")
        .unwrap()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::eq(concat!(
            "cfbench 0.3.0 (Cloudflare Speedtest v1.12.1, ",
            "567aeade7b6e1fbeea98edddb6031c5877678866)\n"
        )));
}

#[test]
fn help_and_version_never_emit_runtime_progress() {
    for argument in ["--help", "--version"] {
        Command::cargo_bin("cfbench")
            .unwrap()
            .arg(argument)
            .assert()
            .success()
            .stderr(predicate::str::is_empty());
    }
}
