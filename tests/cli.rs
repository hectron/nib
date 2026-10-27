use assert_cmd::cargo::cargo_bin_cmd;
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicUsize, Ordering};

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
        .env_remove("NIB_TASK_PARSE_INLINE")
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

fn config_command(root: &std::path::Path) -> assert_cmd::Command {
    let mut command = command();
    command
        .current_dir(root)
        .env("NIB_DEMO", "1")
        .env("XDG_CONFIG_HOME", root.join("xdg"));
    command
}

fn config_root() -> std::path::PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let root = std::env::temp_dir().join(format!(
        "nib-config-cli-test-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&root).expect("config test directory should be created");
    std::fs::canonicalize(root).expect("config test directory should be canonicalized")
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
fn config_help() {
    let out = nib(&["config", "--help"]);
    assert!(out.status.success());
    insta::assert_snapshot!(String::from_utf8(out.stdout).expect("stdout should be UTF-8"));
}

#[test]
fn config_commands_read_and_write_without_apple_backends() {
    let root = config_root();
    let config_path = root.join("xdg/nib/config.toml");

    let set = config_command(&root)
        .args(["config", "set", "task.list", "Work"])
        .output()
        .expect("config set should start");
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert_eq!(
        String::from_utf8(set.stdout)
            .expect("stdout should be UTF-8")
            .replace(&root.display().to_string(), "$ROOT"),
        "Set task.list in $ROOT/xdg/nib/config.toml\n"
    );
    assert_eq!(
        std::fs::read_to_string(&config_path).expect("config file should exist"),
        "[task]\nlist = \"Work\"\n"
    );

    let get = config_command(&root)
        .args(["config", "get", "task.list", "--json"])
        .output()
        .expect("config get should start");
    assert!(
        get.status.success(),
        "{}",
        String::from_utf8_lossy(&get.stderr)
    );
    insta::assert_snapshot!(String::from_utf8(get.stdout).expect("stdout should be UTF-8"), @r#"
    {
      "id": "task.list",
      "value": "Work"
    }
    "#);

    let list = config_command(&root)
        .args(["config", "list", "--origin", "--json"])
        .output()
        .expect("config list should start");
    assert!(
        list.status.success(),
        "{}",
        String::from_utf8_lossy(&list.stderr)
    );
    let entries: serde_json::Value =
        serde_json::from_slice(&list.stdout).expect("config list should be JSON");
    let task_list = entries
        .as_array()
        .expect("config list should be an array")
        .iter()
        .find(|entry| entry["id"] == "task.list")
        .expect("task.list should be present");
    assert_eq!(task_list["value"], "Work");
    assert_eq!(task_list["origin"]["kind"], "file");
    assert_eq!(
        task_list["origin"]["path"],
        config_path.display().to_string()
    );

    let unset = config_command(&root)
        .args(["config", "unset", "task.list"])
        .output()
        .expect("config unset should start");
    assert!(
        unset.status.success(),
        "{}",
        String::from_utf8_lossy(&unset.stderr)
    );
    let missing = config_command(&root)
        .args(["config", "get", "task.list"])
        .output()
        .expect("config get should start");
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(missing.stdout, Vec::<u8>::new());
    assert_eq!(missing.stderr, Vec::<u8>::new());

    let missing_json = config_command(&root)
        .args(["config", "get", "task.list", "--json"])
        .output()
        .expect("JSON config get should start");
    assert_eq!(missing_json.status.code(), Some(1));
    insta::assert_snapshot!(String::from_utf8(missing_json.stdout).expect("stdout should be UTF-8"), @r#"
    {
      "id": "task.list",
      "value": null
    }
    "#);

    std::fs::remove_dir_all(&root).expect("config test directory should be removed");
}

#[test]
fn config_project_target_and_path_json() {
    let root = config_root();
    let set = config_command(&root)
        .args([
            "config",
            "set",
            "cal.calendars",
            "[\"Work\", \"Home\"]",
            "--project",
            "--json",
        ])
        .output()
        .expect("config set should start");
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    let normalized = String::from_utf8(set.stdout)
        .expect("stdout should be UTF-8")
        .replace(&root.display().to_string(), "$ROOT");
    insta::assert_snapshot!(normalized, @r#"
    {
      "id": "cal.calendars",
      "value": [
        "Work",
        "Home"
      ],
      "path": "$ROOT/nib.toml"
    }
    "#);

    let local = config_command(&root)
        .args(["config", "set", "note.folder", "Notes", "--local"])
        .output()
        .expect("local config set should start");
    assert!(
        local.status.success(),
        "{}",
        String::from_utf8_lossy(&local.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(root.join(".nib.local.toml")).expect("local config should exist"),
        "[note]\nfolder = \"Notes\"\n"
    );

    let path = config_command(&root)
        .args(["config", "path", "--json"])
        .output()
        .expect("config path should start");
    assert!(
        path.status.success(),
        "{}",
        String::from_utf8_lossy(&path.stderr)
    );
    let normalized = String::from_utf8(path.stdout)
        .expect("stdout should be UTF-8")
        .replace(&root.display().to_string(), "$ROOT");
    insta::assert_snapshot!(normalized, @r#"
    {
      "id": "user",
      "path": "$ROOT/xdg/nib/config.toml"
    }
    "#);

    std::fs::remove_dir_all(&root).expect("config test directory should be removed");
}

#[test]
fn config_project_target_reuses_hidden_project_file() {
    let root = config_root();
    let hidden = root.join(".nib.toml");
    std::fs::write(
        &hidden,
        "[task]\nlist = \"Team\"\n[note]\nfolder = \"Team Notes\"\n",
    )
    .expect("hidden project config should be written");

    let set = config_command(&root)
        .args(["config", "set", "task.priority", "high", "--project"])
        .output()
        .expect("project config set should start");
    assert!(
        set.status.success(),
        "{}",
        String::from_utf8_lossy(&set.stderr)
    );
    assert!(!root.join("nib.toml").exists());
    let source = std::fs::read_to_string(&hidden).expect("hidden project config should exist");
    assert!(source.contains("list = \"Team\""), "{source}");
    assert!(source.contains("priority = \"high\""), "{source}");
    assert!(source.contains("folder = \"Team Notes\""), "{source}");

    let unset = config_command(&root)
        .args(["config", "unset", "task.list", "--project"])
        .output()
        .expect("project config unset should start");
    assert!(
        unset.status.success(),
        "{}",
        String::from_utf8_lossy(&unset.stderr)
    );
    assert!(!root.join("nib.toml").exists());
    let source = std::fs::read_to_string(&hidden).expect("hidden project config should exist");
    assert!(!source.contains("list ="), "{source}");
    assert!(source.contains("priority = \"high\""), "{source}");

    std::fs::remove_dir_all(&root).expect("config test directory should be removed");
}

#[test]
fn config_unset_missing_key_is_a_no_op_and_set_warns_about_override() {
    let root = config_root();
    let missing = root.join("xdg/nib/config.toml");
    let unset = config_command(&root)
        .args(["config", "unset", "task.list"])
        .output()
        .expect("config unset should start");
    assert!(unset.status.success());
    let normalized = String::from_utf8(unset.stdout)
        .expect("stdout should be UTF-8")
        .replace(&root.display().to_string(), "$ROOT");
    assert_eq!(
        normalized,
        "task.list isn't set in $ROOT/xdg/nib/config.toml; nothing changed\n"
    );
    assert!(!missing.parent().unwrap().exists());

    std::fs::write(root.join("nib.toml"), "[task]\nlist = \"Project\"\n")
        .expect("project config should be written");
    let set = config_command(&root)
        .args(["config", "set", "task.list", "User"])
        .output()
        .expect("config set should start");
    assert!(set.status.success());
    let normalized = String::from_utf8(set.stderr)
        .expect("stderr should be UTF-8")
        .replace(&root.display().to_string(), "$ROOT");
    assert_eq!(
        normalized,
        "warning: task.list is still overridden by $ROOT/nib.toml\n"
    );

    std::fs::remove_dir_all(&root).expect("config test directory should be removed");
}

#[test]
fn config_new_files_honor_the_umask() {
    let root = config_root();
    let binary = assert_cmd::cargo::cargo_bin!("nib");
    let output = std::process::Command::new("sh")
        .args([
            "-c",
            "umask 077; exec \"$1\" config set task.list Work",
            "nib-config-umask",
        ])
        .arg(binary)
        .current_dir(&root)
        .env("NO_COLOR", "1")
        .env("NIB_DEMO", "1")
        .env("XDG_CONFIG_HOME", root.join("xdg"))
        .env_remove("NIB_TASK_LIST")
        .env_remove("NIB_TASK_PRIORITY")
        .env_remove("NIB_TASK_PARSE_INLINE")
        .env_remove("NIB_CAL_CALENDARS")
        .env_remove("NIB_NOTE_FOLDER")
        .output()
        .expect("config set should start under a restrictive umask");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let path = root.join("xdg/nib/config.toml");
    let mode = std::fs::metadata(path)
        .expect("new config should exist")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);

    std::fs::remove_dir_all(&root).expect("config test directory should be removed");
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
fn demo_backend_shows_today_agenda() {
    let out = command()
        .arg("today")
        .env("NIB_DEMO", "1")
        .env("TZ", "UTC0")
        .output()
        .expect("the nib test process should start");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8(out.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("Due today"), "{stdout}");
    assert!(stdout.contains("Events"), "{stdout}");
    assert!(stdout.contains("Review pull request"), "{stdout}");
    assert!(stdout.contains("Design review"), "{stdout}");
}
