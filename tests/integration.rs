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
fn help_lists_all_documented_flags() {
    cmd()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("--icmp"))
        .stdout(predicate::str::contains("--udp"))
        .stdout(predicate::str::contains("--tcp-connect"))
        .stdout(predicate::str::contains("--no-dns"))
        .stdout(predicate::str::contains("--csv"))
        .stdout(predicate::str::contains("--json"))
        .stdout(predicate::str::contains("--max-hops"))
        .stdout(predicate::str::contains("--timeout"))
        .stdout(predicate::str::contains("--size"))
        .stdout(predicate::str::contains("--count"))
        .stdout(predicate::str::contains("--port"));
}

#[test]
fn help_text_mentions_add_and_remove_target() {
    // Verify the in-app help text (compiled into the binary) includes new keybindings.
    // This is a compile-time check via the latensee library.
    let help = latensee::tui::widgets::help::help_text();
    assert!(help.contains("Add target"), "help should mention 'Add target'");
    assert!(help.contains("Remove target"), "help should mention 'Remove target'");
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
        .stdout(predicate::str::contains("hop,host,loss_pct,sent,received,errors,last_ms,avg_ms,best_ms,worst_ms,stdev_ms"));
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
