use assert_cmd::cargo::cargo_bin_cmd;

fn nib(args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("nib")
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn help() {
    let out = nib(&["--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).unwrap());
}

#[test]
fn task_help() {
    let out = nib(&["task", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).unwrap());
}

#[test]
fn stub_backend_fails_cleanly() {
    let out = nib(&["n"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("Apple Notes integration is not implemented yet"),
        "{stderr}"
    );
}

#[test]
fn demo_backend_shows_sample_data() {
    let out = cargo_bin_cmd!("nib")
        .arg("t")
        .env("NO_COLOR", "1")
        .env("NIB_DEMO", "1")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(stdout.starts_with("TITLE"), "{stdout}");
    assert!(stdout.contains("Review pull request"), "{stdout}");
}

#[test]
fn stub_backend_error_has_a_code() {
    let out = nib(&["n"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("nib::backend::not_implemented"), "{stderr}");
}
