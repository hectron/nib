use clap::CommandFactory;
use clap_complete::{Shell, generate};

use super::{Cli, CompletionShell};

pub fn script(shell: CompletionShell) -> String {
    let shell = Shell::from(shell);
    let mut command = Cli::command();
    let mut generated = Vec::new();
    generate(shell, &mut command, "nib", &mut generated);
    let generated = String::from_utf8(generated)
        .expect("clap completion generators always produce UTF-8 shell scripts");
    match shell {
        Shell::Bash => bash(&generated),
        Shell::Fish => fish(&generated),
        Shell::Zsh => zsh(&generated),
        _ => unreachable!("nib exposes only zsh, bash, and fish completion generators"),
    }
}

fn bash(generated: &str) -> String {
    let generated = hide_bash_completion_command(generated);
    let mut script = generated.replacen("_nib() {", "_nib_clap() {", 1);
    script.push_str(BASH_WRAPPER);
    script
}

const BASH_WRAPPER: &str = r#"
_nib() {
    local cur="$2"
    local prev="$3"
    local word="${COMP_WORDS[COMP_CWORD]}"
    local option="$prev"
    local current="$cur"
    local prefix=""
    local split_colon=""
    local command_name=""
    local subcommand=""
    local index
    local command_word

    for (( index = 1; index < COMP_CWORD; index += 1 )); do
        command_word="${COMP_WORDS[index]}"
        if [[ "$command_word" == -* ]]; then
            continue
        fi
        if [[ -z "$command_name" ]]; then
            command_name="$command_word"
        elif [[ -z "$subcommand" ]]; then
            subcommand="$command_word"
            break
        fi
    done

    case "$word" in
        --list=*)
            option="--list"
            if [[ "$word" == "$cur" ]]; then
                current="${cur#*=}"
                prefix="--list="
            elif [[ "$word" == *:* ]]; then
                current="${word#*=}"
                split_colon=1
            fi
            ;;
        --calendar=*)
            option="--calendar"
            if [[ "$word" == "$cur" ]]; then
                current="${cur#*=}"
                prefix="--calendar="
            elif [[ "$word" == *:* ]]; then
                current="${word#*=}"
                split_colon=1
            fi
            ;;
    esac
    if [[ "$prev" == "=" && $COMP_CWORD -ge 2 ]]; then
        option="${COMP_WORDS[COMP_CWORD-2]}"
    elif [[ "$word" == *:* && "$word" != "$cur" && "$word" != --list=* && "$word" != --calendar=* ]]; then
        current="$word"
        option="${COMP_WORDS[COMP_CWORD-1]}"
        split_colon=1
    elif [[ "$prev" == ":" && $COMP_CWORD -ge 3 ]]; then
        current="${COMP_WORDS[COMP_CWORD-2]}:${cur}"
        option="${COMP_WORDS[COMP_CWORD-3]}"
        split_colon=1
    fi

    local kind=""
    case "$option,$command_name,$subcommand" in
        --list,t,*|-l,t,*|--list,task,add|-l,task,add|--list,todo,add|-l,todo,add)
            kind="writable-reminder-lists"
            ;;
        --list,task,edit|-l,task,edit|--list,todo,edit|-l,todo,edit)
            kind="reminder-lists"
            ;;
        --calendar,today,*|--calendar,cal,*|--calendar,c,*)
            kind="calendars"
            ;;
        --folder,n,*|-f,n,*|--folder,note,*|-f,note,*)
            kind="note-folders"
            ;;
    esac
    if [[ -z "$kind" ]]; then
        _nib_clap "$@"
        return
    fi
    COMPREPLY=()
    local candidate
    local reply
    current="${current//\\/}"
    while IFS= read -r candidate; do
        if [[ "$candidate" == "$current"* ]]; then
            reply="$candidate"
            if [[ -n "$prefix" ]]; then
                reply="$prefix$reply"
            elif [[ -n "$split_colon" ]]; then
                reply="${reply##*:}"
            fi
            reply="$(printf '%q' "$reply")"
            if [[ "$reply" == "~"* ]]; then
                reply="\\$reply"
            fi
            COMPREPLY+=("$reply")
        fi
    done < <(command nib __complete "$kind" 2>/dev/null)
}
complete -F _nib nib
"#;

fn hide_bash_completion_command(generated: &str) -> String {
    let mut script = String::with_capacity(generated.len());
    let mut hidden_arm = None;
    for line in generated.lines() {
        let trimmed = line.trim();
        if trimmed.ends_with(')') && trimmed.contains("__complete)") {
            hidden_arm = Some(line.len() - line.trim_start().len());
            continue;
        }
        if let Some(indentation) = hidden_arm {
            if trimmed == ";;" && line.len() - line.trim_start().len() == indentation + 4 {
                hidden_arm = None;
            }
            continue;
        }
        let line = line
            .replacen(
                " --version __complete completions",
                " --version completions",
                1,
            )
            .replacen("opts=\"__complete completions", "opts=\"completions", 1);
        script.push_str(&line);
        script.push('\n');
    }
    script
}

fn fish(generated: &str) -> String {
    let mut script = String::with_capacity(generated.len() + 512);
    for line in generated.lines() {
        if line.contains("-a \"__complete\"")
            || line.contains("__fish_nib_using_subcommand __complete")
        {
            continue;
        }
        script.push_str(line);
        let kind = if has_long_option(line, "list") {
            if line.contains("__fish_seen_subcommand_from edit") {
                Some("reminder-lists")
            } else {
                Some("writable-reminder-lists")
            }
        } else if has_long_option(line, "calendar") {
            Some("calendars")
        } else if has_long_option(line, "folder") {
            Some("note-folders")
        } else {
            None
        };
        if let Some(kind) = kind {
            script.push_str(" -f -a '(command nib __complete ");
            script.push_str(kind);
            script.push_str(" 2>/dev/null)'");
        }
        script.push('\n');
    }
    script
}

fn has_long_option(line: &str, name: &str) -> bool {
    let words = line.split_whitespace().collect::<Vec<_>>();
    line.starts_with("complete -c nib ")
        && words
            .windows(2)
            .any(|pair| pair.first() == Some(&"-l") && pair.get(1) == Some(&name))
}

fn zsh(generated: &str) -> String {
    let generated = generated
        .lines()
        .filter(|line| line.trim_start() != "'__complete:' \\")
        .collect::<Vec<_>>()
        .join("\n");
    let generated = generated.replacen("_nib() {", "_nib_clap() {", 1);
    let wrapper = r#"_nib() {
    local option="${words[CURRENT-1]}"
    local current="${words[CURRENT]}"
    local prefix=""
    local command_name=""
    local subcommand=""
    local index
    local command_word
    for (( index = 2; index < CURRENT; index += 1 )); do
        command_word="${words[index]}"
        if [[ "$command_word" == -* ]]; then
            continue
        fi
        if [[ -z "$command_name" ]]; then
            command_name="$command_word"
        elif [[ -z "$subcommand" ]]; then
            subcommand="$command_word"
            break
        fi
    done
    case "$current" in
        --list=*) option="--list"; current="${current#*=}"; prefix="--list=" ;;
        --calendar=*) option="--calendar"; current="${current#*=}"; prefix="--calendar=" ;;
    esac

    local kind=""
    case "$option,$command_name,$subcommand" in
        --list,t,*|-l,t,*|--list,task,add|-l,task,add|--list,todo,add|-l,todo,add)
            kind="writable-reminder-lists"
            ;;
        --list,task,edit|-l,task,edit|--list,todo,edit|-l,todo,edit)
            kind="reminder-lists"
            ;;
        --calendar,today,*|--calendar,cal,*|--calendar,c,*)
            kind="calendars"
            ;;
        --folder,n,*|-f,n,*|--folder,note,*|-f,note,*)
            kind="note-folders"
            ;;
    esac
    if [[ -z "$kind" ]]; then
        _nib_clap "$@"
        return
    fi
    local -a candidates
    candidates=(${(f)"$(command nib __complete "$kind" 2>/dev/null)"})
    if (( $#candidates )) && [[ -n "$prefix" ]]; then
        for (( index = 1; index <= $#candidates; index += 1 )); do
            candidates[index]="$prefix$candidates[index]"
        done
    fi
    (( $#candidates )) && compadd -a candidates
}
"#;
    let trailer = "if [ \"$funcstack[1]\" = \"_nib\" ]; then";
    generated.replacen(trailer, &format!("{wrapper}\n{trailer}"), 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fish_keeps_clap_conditions_when_adding_live_values() {
        let generated = "complete -c nib -n 'condition' -l list -r\ncomplete -c nib -l help\n";
        let script = fish(generated);
        let expected = "complete -c nib -n 'condition' -l list -r -f -a '(command nib __complete writable-reminder-lists 2>/dev/null)'";
        assert_eq!(script.lines().next(), Some(expected));
        assert_eq!(script.lines().nth(1), Some("complete -c nib -l help"));
    }

    #[test]
    fn bash_removes_only_the_hidden_completion_command() {
        let generated = r#"case "$cmd" in
    nib,__complete)
        cmd="nib__subcmd____complete"
        ;;
    nib,completions)
        cmd="nib__subcmd__completions"
        ;;
esac
opts="-V --version __complete completions"
opts="__complete completions doctor"
case "$cmd" in
    nib__subcmd____complete)
        opts="--refresh"
        ;;
    nib__subcmd__completions)
        opts="zsh bash fish"
        ;;
esac
"#;
        let script = hide_bash_completion_command(generated);
        assert!(!script.contains("nib,__complete)"));
        assert!(!script.contains("nib__subcmd____complete)"));
        assert!(!script.contains("--version __complete completions"));
        assert!(!script.contains("opts=\"__complete completions"));
        assert!(script.contains("nib,completions)"));
        assert!(script.contains("nib__subcmd__completions)"));
    }
}
