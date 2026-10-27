use assert_cmd::cargo::cargo_bin_cmd;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

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
            "[completions|doctor|today|note|task|cal|n|t|help|]\n",
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
            "[completions|doctor|today|note|task|todo|cal|c|n|t|help]\n",
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
