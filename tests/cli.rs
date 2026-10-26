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
    let out = nib(&["cal"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(
        stderr.contains("Calendar integration is not implemented yet"),
        "{stderr}"
    );
}

#[test]
fn stub_backend_error_has_a_code() {
    let out = nib(&["cal"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).unwrap();
    assert!(stderr.contains("nib::backend::not_implemented"), "{stderr}");
}
