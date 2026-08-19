use std::path::PathBuf;
use std::process::Command;

fn fixture(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/fixtures")
        .join(name)
}

#[test]
fn prints_mir_for_local_functions() {
    let output = Command::new(env!("CARGO_BIN_EXE_omlua-driver"))
        .arg(fixture("branch.rs"))
        .output()
        .expect("failed to run omlua-driver");

    assert!(output.status.success());

    let stdout = String::from_utf8(output.stdout).expect("MIR output is not UTF-8");
    assert!(stdout.contains("fn classify(_1: i32) -> i32"));
    assert!(stdout.contains("fn main() -> ()"));
    assert!(stdout.contains("bb0:"));
}

#[test]
fn preserves_rustc_failure_status_and_diagnostic() {
    let output = Command::new(env!("CARGO_BIN_EXE_omlua-driver"))
        .arg(fixture("type_error.rs"))
        .output()
        .expect("failed to run omlua-driver");

    assert!(!output.status.success());

    let stderr = String::from_utf8(output.stderr).expect("rustc diagnostic is not UTF-8");
    assert!(stderr.contains("error[E0308]: mismatched types"));
    assert!(stderr.contains("expected `i32`, found `&str`"));
}
