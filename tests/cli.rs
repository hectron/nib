use assert_cmd::cargo::cargo_bin_cmd;

fn command() -> assert_cmd::Command {
    let mut command = cargo_bin_cmd!("nib");
    command
        .env("NO_COLOR", "1")
        .env(
            "XDG_CONFIG_HOME",
            std::env::temp_dir().join(format!("nib-cli-test-{}", std::process::id())),
        )
        .env_remove("NIB_TASK_LIST")
        .env_remove("NIB_TASK_PRIORITY")
        .env_remove("NIB_CAL_CALENDARS")
        .env_remove("NIB_NOTE_FOLDER");
    command
}

fn nib(args: &[&str]) -> std::process::Output {
    command()
        .args(args)
        .output()
        .expect("the nib test process should start")
}

#[test]
fn help() {
    let out = nib(&["--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn task_help() {
    let out = nib(&["task", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn task_capture_help() {
    let out = nib(&["t", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn task_add_help() {
    let out = nib(&["task", "add", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn calendar_help() {
    let out = nib(&["cal", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn stub_backend_fails_cleanly() {
    let out = nib(&["n"]);
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8(out.stderr).expect("stderr should be UTF-8");
    assert!(
        stderr.contains("Apple Notes integration is not implemented yet"),
        "{stderr}"
    );
}

#[test]
fn demo_backend_shows_sample_data() {
    let out = command()
        .arg("t")
        .env("NIB_DEMO", "1")
        .output()
        .expect("the nib test process should start");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("stdout should be UTF-8");
    assert!(stdout.starts_with("TITLE"), "{stdout}");
    assert!(stdout.contains("Review pull request"), "{stdout}");
}

#[test]
fn stub_backend_error_has_a_code() {
    let out = nib(&["n"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8(out.stderr).expect("stderr should be UTF-8");
    assert!(stderr.contains("nib::backend::not_implemented"), "{stderr}");
}
