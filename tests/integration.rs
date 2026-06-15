use assert_cmd::Command;
use predicates::prelude::*;

fn cmd() -> Command {
    Command::cargo_bin("latensee").unwrap()
}

#[test]
fn help_exits_zero_and_contains_expected_text() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("latensee"))
        .stdout(predicate::str::contains("Target hostnames or IP addresses"))
        .stdout(predicate::str::contains("--interval"))
        .stdout(predicate::str::contains("--report"));
}

#[test]
fn version_exits_zero() {
    cmd()
        .arg("--version")
        .assert()
        .success()
        .stdout(predicate::str::contains("latensee"));
}

#[test]
fn missing_target_exits_nonzero() {
    cmd()
        .assert()
        .failure()
        .stderr(predicate::str::contains("required"));
}

#[test]
fn invalid_interval_zero_exits_nonzero() {
    cmd()
        .args(["example.com", "-i", "0"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("interval"));
}

#[test]
fn invalid_size_too_small_exits_nonzero() {
    cmd()
        .args(["example.com", "-s", "10"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("28"));
}

#[test]
#[ignore = "needs network and UDP socket"]
fn report_mode_udp_localhost() {
    cmd()
        .args(["127.0.0.1", "--udp", "--report", "-c", "2", "-i", "0.1", "-t", "0.5"])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success()
        .stdout(predicate::str::contains("latensee report"));
}

#[test]
#[ignore = "needs network and UDP socket"]
fn csv_output_contains_header() {
    cmd()
        .args(["127.0.0.1", "--udp", "--csv", "-c", "1", "-i", "0.1", "-t", "0.5"])
        .timeout(std::time::Duration::from_secs(10))
        .assert()
        .success()
        .stdout(predicate::str::contains("hop,host,loss_pct,sent,received,last_ms,avg_ms,best_ms,worst_ms,stdev_ms"));
}

#[test]
#[ignore = "needs network and UDP socket"]
fn json_output_is_valid_json() {
    let output = cmd()
        .args(["127.0.0.1", "--udp", "--json", "-c", "1", "-i", "0.1", "-t", "0.5"])
        .timeout(std::time::Duration::from_secs(10))
        .output()
        .expect("failed to execute");

    assert!(output.status.success(), "exit code should be 0");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let parsed: serde_json::Value =
        serde_json::from_str(&stdout).expect("stdout should be valid JSON");

    assert!(parsed.get("target").is_some());
    assert!(parsed.get("hops").is_some());
}
