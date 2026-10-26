use assert_cmd::cargo::cargo_bin_cmd;

fn jot(args: &[&str]) -> std::process::Output {
    cargo_bin_cmd!("jot")
        .args(args)
        .env("NO_COLOR", "1")
        .output()
        .unwrap()
}

#[test]
fn help() {
    let out = jot(&["--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).unwrap());
}

#[test]
fn task_help() {
    let out = jot(&["task", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).unwrap());
}

#[test]
fn stub_backend_fails_cleanly() {
    let out = jot(&["cal"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("Calendar integration is not implemented yet"),
        "{stderr}"
    );
}
