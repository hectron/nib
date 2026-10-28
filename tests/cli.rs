use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};

static TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let id = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
        let path =
            std::env::temp_dir().join(format!("nib-shell-{name}-{}-{id}", std::process::id()));
        fs::create_dir_all(&path).expect("the shell test directory should be created");
        Self(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

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
        .env_remove("NIB_NOTE_FOLDER")
        .env_remove("NIB_DEMO");
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

fn completion_script(shell: &str) -> String {
    let out = nib(&["completions", shell]);
    assert!(
        out.status.success(),
        "{shell}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("stdout should be UTF-8")
}

fn write_stub(directory: &Path) {
    let stub = directory.join("nib");
    fs::write(
        &stub,
        r#"#!/bin/sh
[ "$NIB_STUB_EMPTY" = "1" ] && exit 0
case "$2" in
    writable-reminder-lists)
        printf '%s\n' 'Work Projects' "Bob's list" 'Q4: Plan' 'Say "hi"' '$HOME' '*glob' '#bills' '~tilde' 'Alpha:Beta'
        ;;
    reminder-lists)
        printf '%s\n' 'Work Projects' 'Read Only'
        ;;
    calendars)
        printf '%s\n' 'Work Calendar'
        ;;
    note-folders)
        printf '%s\n' 'Folder "One"'
        ;;
esac
"#,
    )
    .expect("the nib shell stub should be written");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&stub)
            .expect("the nib shell stub should exist")
            .permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(stub, permissions).expect("the nib shell stub should be executable");
    }
}

fn run_shell(shell: &str, script: &str, path: &Path) -> std::process::Output {
    let mut child = Command::new(shell)
        .env(
            "PATH",
            format!(
                "{}:{}",
                path.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the shell test process should start");
    child
        .stdin
        .take()
        .expect("the shell test process should have piped stdin")
        .write_all(script.as_bytes())
        .expect("the shell harness should be written");
    child
        .wait_with_output()
        .expect("the shell test process should finish")
}

#[test]
fn zsh_quoted_quick_add_preserves_an_apostrophe_and_bang_token() {
    if !Path::new("/bin/zsh").is_file() {
        return;
    }
    let temporary = TempDir::new("zsh-quick-add-quotes");
    let stub = temporary.0.join("nib");
    fs::write(&stub, "#!/bin/sh\nprintf '<%s>\\n' \"$@\"\n")
        .expect("the nib argument stub should be written");
    let mut permissions = fs::metadata(&stub)
        .expect("the nib argument stub should exist")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&stub, permissions).expect("the nib argument stub should be executable");

    let output = run_shell(
        "/bin/zsh",
        "nib t \"Get Apple Cider from the Farmer's Market\" tomorrow '!high'\n",
        &temporary.0,
    );
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("zsh output should be UTF-8"),
        "<t>\n<Get Apple Cider from the Farmer's Market>\n<tomorrow>\n<!high>\n"
    );
}

#[test]
fn bash_completion_behavior() {
    if !Path::new("/bin/bash").is_file() {
        return;
    }
    let temporary = TempDir::new("bash");
    write_stub(&temporary.0);
    let mut script = completion_script("bash");
    assert!(!script.contains("compopt -o filenames"));
    assert!(!script.contains("-o filenames"));
    script.push_str(
        r#"
emit() {
    printf '['
    if (( ${#COMPREPLY[@]} )); then printf '%s|' "${COMPREPLY[@]}"; fi
    printf ']\n'
}
run() {
    COMP_WORDS=("$@")
    COMP_CWORD=$(( ${#COMP_WORDS[@]} - 1 ))
    COMP_LINE="$*"
    COMP_POINT=${#COMP_LINE}
    local cur="${COMP_WORDS[COMP_CWORD]}"
    local prev="${COMP_WORDS[COMP_CWORD-1]}"
    COMPREPLY=()
    _nib nib "$cur" "$prev"
    emit
}
run_readline() {
    local cur="$1"
    local prev="$2"
    shift 2
    COMP_WORDS=("$@")
    COMP_CWORD=$(( ${#COMP_WORDS[@]} - 1 ))
    COMPREPLY=()
    _nib nib "$cur" "$prev"
    emit
}
run nib ta
run nib help ""
run nib config get ta
run nib t title --priority ""
run nib t title --list ""
run nib t title --list 'Work\ P'
run_readline Wo title nib t title --list=Wo
run_readline Wo cal nib cal --calendar=Wo
run_readline "" --list nib t title --list Q4:
run_readline "" --list nib t title --list Alpha:
run_readline '\~t' --list nib t title --list '\~t'
run nib --json t title --list W
run nib task edit --list ""
run nib note list --folder ""
run nib t title -f ""
export NIB_STUB_EMPTY=1
run nib t title --list ""
complete -p nib
"#,
    );
    let output = run_shell("/bin/bash", &script, &temporary.0);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("bash output should be UTF-8"),
        concat!(
            "[task|]\n",
            "[completions|doctor|config|today|note|task|cal|n|t|log|undo|help|]\n",
            "[task.list|task.priority|task.parse_inline|]\n",
            "[low|medium|high|]\n",
            "[Work\\ Projects|Bob\\'s\\ list|Q4:\\ Plan|Say\\ \\\"hi\\\"|\\$HOME|\\*glob|\\#bills|\\~tilde|Alpha:Beta|]\n",
            "[Work\\ Projects|]\n",
            "[Work\\ Projects|]\n",
            "[Work\\ Calendar|]\n",
            "[\\ Plan|]\n",
            "[Beta|]\n",
            "[\\~tilde|]\n",
            "[Work\\ Projects|]\n",
            "[Work\\ Projects|Read\\ Only|]\n",
            "[Folder\\ \\\"One\\\"|]\n",
            "[-l|-d|-p|-h|--list|--due|--priority|--json|--help|]\n",
            "[]\n",
            "complete -F _nib nib\n",
        )
    );
}

#[test]
fn zsh_completion_behavior_and_fpath_autoload() {
    if !Path::new("/bin/zsh").is_file() {
        return;
    }
    let temporary = TempDir::new("zsh");
    write_stub(&temporary.0);
    let generated = completion_script("zsh");
    let mut script = String::from(
        r#"autoload() { :; }
is-at-least() { return 0; }
compdef() { :; }
typeset -ga captured
compadd() { shift; captured=("${(@P)1}"); }
_describe() { local -a values; values=("${(@P)4}"); captured=("${(@)values%%:*}"); }
_arguments() { return 1; }
"#,
    );
    script.push_str(&generated);
    script.push_str(
        r#"
emit() { print -r -- "[${(j:|:)captured}]"; captured=(); }
_nib_commands; emit
_arguments() {
    local spec
    for spec in "$@"; do
        if [[ "$spec" == *':PRIORITY:(low medium high)'* ]]; then
            captured=(low medium high)
            return 0
        fi
    done
    state=nib
    line=(t)
    return 1
}
words=(nib t title --priority ""); CURRENT=5; _nib; emit
_arguments() { return 1; }
words=(nib t title --list ""); CURRENT=5; _nib; emit
words=(nib task edit --list ""); CURRENT=5; _nib; emit
words=(nib note list --folder ""); CURRENT=5; _nib; emit
words=(nib t title "--list="); CURRENT=4; _nib; emit
words=(nib cal "--calendar="); CURRENT=3; _nib; emit
words=(nib --json t title --list ""); CURRENT=6; _nib; emit
words=(nib t title -f ""); CURRENT=5; _nib; emit
export NIB_STUB_EMPTY=1
words=(nib t title --list ""); CURRENT=5; _nib; emit
"#,
    );
    let output = run_shell("/bin/zsh", &script, &temporary.0);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("zsh output should be UTF-8"),
        concat!(
            "[completions|doctor|config|today|note|task|todo|cal|c|n|t|log|undo|help]\n",
            "[low|medium|high]\n",
            "[Work Projects|Bob's list|Q4: Plan|Say \"hi\"|$HOME|*glob|#bills|~tilde|Alpha:Beta]\n",
            "[Work Projects|Read Only]\n",
            "[Folder \"One\"]\n",
            "[--list=Work Projects|--list=Bob's list|--list=Q4: Plan|--list=Say \"hi\"|--list=$HOME|--list=*glob|--list=#bills|--list=~tilde|--list=Alpha:Beta]\n",
            "[--calendar=Work Calendar]\n",
            "[Work Projects|Bob's list|Q4: Plan|Say \"hi\"|$HOME|*glob|#bills|~tilde|Alpha:Beta]\n",
            "[]\n",
            "[]\n",
        )
    );

    let fpath = temporary.0.join("fpath");
    fs::create_dir(&fpath).expect("the zsh fpath should be created");
    fs::write(fpath.join("_nib"), generated).expect("the zsh completion should be installed");
    let autoload = format!(
        r"fpath=({} $fpath)
is-at-least() {{ return 0; }}
compdef() {{ :; }}
compadd() {{ :; }}
_arguments() {{ print -r -- autoload-ok; return 1; }}
words=(nib task)
CURRENT=2
autoload -Uz _nib
_nib
",
        fpath.display()
    );
    let output = run_shell("/bin/zsh", &autoload, &temporary.0);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("zsh output should be UTF-8"),
        "autoload-ok\n"
    );
}

#[test]
fn demo_completion_values_are_live_without_apple_data() {
    let out = command()
        .args(["__complete", "calendars"])
        .env("NIB_DEMO", "1")
        .output()
        .expect("the nib test process should start");
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8(out.stdout).expect("stdout should be UTF-8"),
        "Personal\nWork\n"
    );
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
fn demo_backend_lists_resources_in_text_and_json() {
    for (args, heading) in [
        (&["note", "folders"][..], "ACCOUNT"),
        (&["task", "lists"][..], "SOURCE"),
        (&["cal", "calendars"][..], "SOURCE"),
    ] {
        let out = command()
            .args(args)
            .env("NIB_DEMO", "1")
            .output()
            .expect("the nib test process should start");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("stdout should be UTF-8");
        assert!(stdout.contains(heading), "{args:?}: {stdout}");

        let out = command()
            .args(args)
            .arg("--json")
            .env("NIB_DEMO", "1")
            .output()
            .expect("the nib test process should start");
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).expect("stdout should be UTF-8");
        assert!(stdout.contains("{\n    \"id\":"), "{args:?}: {stdout}");
        let values: serde_json::Value =
            serde_json::from_str(&stdout).expect("resource output should be JSON");
        let first = values
            .as_array()
            .and_then(|values| values.first())
            .and_then(serde_json::Value::as_object)
            .expect("demo resources should contain an object");
        assert!(first.contains_key("id"));
    }
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
