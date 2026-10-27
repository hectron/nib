use std::process::Command;

use jiff::Timestamp;
use serde::Deserialize;

use crate::domain::{BackendError, NewNote, Note, NoteQuery, Notes, terminal_name};

const APP: &str = "Apple Notes";
const FOLDER_AMBIGUOUS: &str = "NIB_FOLDER_AMBIGUOUS:";
const FOLDER_NOT_FOUND: &str = "NIB_FOLDER_NOT_FOUND";
const CREATE_SCRIPT: &str = r#"
function folderNamed(notes, name) {
    const folders = notes.folders.whose({name: name})();
    if (folders.length === 0) {
        throw new Error("NIB_FOLDER_NOT_FOUND");
    }
    if (folders.length > 1) {
        const accounts = notes.accounts()
            .filter(account => account.folders.whose({name: name})().length > 0)
            .map(account => account.name());
        throw new Error("NIB_FOLDER_AMBIGUOUS:" + JSON.stringify(accounts));
    }
    return folders[0];
}

function run(argv) {
    const notes = Application("Notes");
    const folder = argv.length === 1 ? notes.defaultAccount().defaultFolder() : folderNamed(notes, argv[1]);
    const created = notes.Note({body: argv[0]});
    folder.notes.push(created);
    const modified = created.modificationDate();
    return JSON.stringify({
        id: created.id(),
        title: created.name(),
        folder: folder.name(),
        modified: modified === null ? null : modified.toISOString(),
        locked: created.passwordProtected()
    });
}
"#;
const LIST_SCRIPT: &str = r#"
function folderNamed(notes, name) {
    const folders = notes.folders.whose({name: name})();
    if (folders.length === 0) {
        throw new Error("NIB_FOLDER_NOT_FOUND");
    }
    if (folders.length > 1) {
        const accounts = notes.accounts()
            .filter(account => account.folders.whose({name: name})().length > 0)
            .map(account => account.name());
        throw new Error("NIB_FOLDER_AMBIGUOUS:" + JSON.stringify(accounts));
    }
    return folders[0];
}

function append(result, folder, ids, names, dates, locked) {
    for (let index = 0; index < ids.length; index += 1) {
        result.push({
            id: ids[index],
            title: names[index],
            folder: folder,
            modified: dates[index] === null ? null : dates[index].toISOString(),
            locked: locked[index]
        });
    }
}

function run(argv) {
    const notes = Application("Notes");
    const result = [];
    if (argv.length === 0) {
        const folders = notes.folders.name();
        const ids = notes.folders.notes.id();
        const names = notes.folders.notes.name();
        const dates = notes.folders.notes.modificationDate();
        const locked = notes.folders.notes.passwordProtected();
        for (let index = 0; index < folders.length; index += 1) {
            append(result, folders[index], ids[index], names[index], dates[index], locked[index]);
        }
    } else {
        const folder = folderNamed(notes, argv[0]);
        const selected = folder.notes;
        append(result, folder.name(), selected.id(), selected.name(), selected.modificationDate(), selected.passwordProtected());
    }
    return JSON.stringify(result);
}
"#;
#[cfg(all(test, target_os = "macos"))]
const DELETE_SCRIPT: &str = r#"
function run(argv) {
    const notes = Application("Notes");
    const matches = notes.notes.whose({id: argv[0]})();
    if (matches.length !== 0) {
        matches[0].delete();
    }
}
"#;

pub struct AppleNotes {
    terminal: String,
}

impl AppleNotes {
    pub fn new() -> Self {
        Self {
            terminal: terminal_name(std::env::var("TERM_PROGRAM").ok().as_deref()),
        }
    }

    fn execute<T: for<'de> Deserialize<'de>>(
        &self,
        script: &str,
        args: &[String],
        folder: Option<&str>,
    ) -> Result<T, BackendError> {
        let output = run_script(script, args)?;
        parse(&output, folder, &self.terminal)
    }

    #[cfg(all(test, target_os = "macos"))]
    fn delete(&self, id: &str) -> Result<(), BackendError> {
        let output = run_script(DELETE_SCRIPT, &[id.into()])?;
        if output.success {
            Ok(())
        } else {
            Err(script_error(&output.stderr, None, &self.terminal))
        }
    }
}

impl Notes for AppleNotes {
    fn create(&self, note: NewNote) -> Result<Note, BackendError> {
        let args = create_arguments(&note)?;
        let created: ScriptNote = self.execute(CREATE_SCRIPT, &args, note.folder.as_deref())?;
        Ok(created.into())
    }

    fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError> {
        let args: Vec<String> = query.folder.iter().cloned().collect();
        let notes: Vec<ScriptNote> = self.execute(LIST_SCRIPT, &args, query.folder.as_deref())?;
        let mut notes: Vec<Note> = notes.into_iter().map(Into::into).collect();
        notes.sort_by(|a: &Note, b: &Note| {
            b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id))
        });
        Ok(notes)
    }
}

fn create_arguments(note: &NewNote) -> Result<Vec<String>, BackendError> {
    let mut args = vec![html_body(&note.title)?];
    if let Some(folder) = &note.folder {
        args.push(folder.clone());
    }
    Ok(args)
}

struct ScriptOutput {
    success: bool,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

fn run_script(script: &str, args: &[String]) -> Result<ScriptOutput, BackendError> {
    let output = script_command(script, args)
        .output()
        .map_err(|error| BackendError::Failed {
            app: APP,
            message: error.to_string(),
        })?;
    Ok(ScriptOutput {
        success: output.status.success(),
        stdout: output.stdout,
        stderr: output.stderr,
    })
}

fn script_command(script: &str, args: &[String]) -> Command {
    let mut command = Command::new("osascript");
    command
        .args(["-l", "JavaScript", "-e", script, "--"])
        .args(args);
    command
}

fn parse<T: for<'de> Deserialize<'de>>(
    output: &ScriptOutput,
    folder: Option<&str>,
    terminal: &str,
) -> Result<T, BackendError> {
    if !output.success {
        return Err(script_error(&output.stderr, folder, terminal));
    }
    serde_json::from_slice(&output.stdout).map_err(|error| BackendError::Failed {
        app: APP,
        message: format!("couldn't understand the response from Notes: {error}"),
    })
}

fn script_error(stderr: &[u8], folder: Option<&str>, terminal: &str) -> BackendError {
    let message = String::from_utf8_lossy(stderr).trim().to_owned();
    if message.contains("-1743") {
        BackendError::AutomationDenied {
            app: APP,
            terminal: terminal.into(),
        }
    } else if message.contains(FOLDER_NOT_FOUND) {
        BackendError::NotFound {
            kind: "folder",
            name: folder.unwrap_or_default().into(),
        }
    } else if let Some(accounts) = ambiguous_accounts(&message) {
        BackendError::Ambiguous {
            kind: "folder",
            name: folder.unwrap_or_default().into(),
            matches: accounts
                .into_iter()
                .map(|account| format!("account \"{account}\""))
                .collect(),
        }
    } else {
        BackendError::Failed { app: APP, message }
    }
}

fn ambiguous_accounts(message: &str) -> Option<Vec<String>> {
    let (_, encoded) = message.split_once(FOLDER_AMBIGUOUS)?;
    let mut deserializer = serde_json::Deserializer::from_str(encoded);
    Vec::deserialize(&mut deserializer).ok()
}

fn html_body(title: &str) -> Result<String, BackendError> {
    if title.starts_with(' ')
        || title.ends_with(' ')
        || title.contains("  ")
        || title
            .chars()
            .any(|character| character.is_whitespace() && character != ' ')
    {
        return Err(BackendError::UnsupportedNoteWhitespace);
    }
    let mut escaped = String::with_capacity(title.len() + 11);
    for character in title.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(character),
        }
    }
    Ok(format!("<div>{escaped}</div>"))
}

#[derive(Debug, Deserialize)]
struct ScriptNote {
    id: String,
    title: String,
    folder: String,
    modified: Option<Timestamp>,
    locked: bool,
}

impl From<ScriptNote> for Note {
    fn from(note: ScriptNote) -> Self {
        Self {
            id: note.id,
            title: note.title,
            folder: Some(note.folder),
            modified: note.modified,
            locked: note.locked,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;

    use super::*;

    fn output(success: bool, stdout: &str, stderr: &str) -> ScriptOutput {
        ScriptOutput {
            success,
            stdout: stdout.into(),
            stderr: stderr.into(),
        }
    }

    #[cfg(target_os = "macos")]
    struct LiveNoteCleanup {
        id: Option<String>,
    }

    #[cfg(target_os = "macos")]
    impl Drop for LiveNoteCleanup {
        fn drop(&mut self) {
            if let Some(id) = self.id.take() {
                let _ = AppleNotes::new().delete(&id);
            }
        }
    }

    #[test]
    fn html_escapes_note_titles() {
        assert_eq!(
            html_body(r#"R&D <plan> "quoted" 'draft' \\ path"#).unwrap(),
            r"<div>R&amp;D &lt;plan&gt; &quot;quoted&quot; &#39;draft&#39; \\ path</div>"
        );
    }

    #[test]
    fn rejects_title_whitespace_that_notes_would_normalize() {
        for title in [" A", "A ", "A  B", "A\tB", "A\nB"] {
            assert!(
                matches!(
                    html_body(title),
                    Err(BackendError::UnsupportedNoteWhitespace)
                ),
                "{title:?}"
            );
        }
    }

    #[test]
    fn note_values_are_separate_script_arguments() {
        let note = NewNote {
            title: r#"A "quote" and \\ path <tag>"#.into(),
            folder: Some(r#"Folder "one" \\"#.into()),
        };
        let args = create_arguments(&note).unwrap();
        assert_eq!(
            args,
            [
                r"<div>A &quot;quote&quot; and \\ path &lt;tag&gt;</div>",
                r#"Folder "one" \\"#,
            ]
        );
        assert!(!CREATE_SCRIPT.contains(&note.title));
        assert!(!CREATE_SCRIPT.contains(note.folder.as_deref().unwrap()));
    }

    #[test]
    fn osascript_preserves_arguments() {
        let args = vec![
            r#"quotes " and backslash \\"#.into(),
            "-leading-dash".into(),
        ];
        let command = script_command("return", &args);
        let command_args: Vec<&OsStr> = command.get_args().collect();
        assert_eq!(
            command_args,
            [
                OsStr::new("-l"),
                OsStr::new("JavaScript"),
                OsStr::new("-e"),
                OsStr::new("return"),
                OsStr::new("--"),
                OsStr::new(&args[0]),
                OsStr::new(&args[1]),
            ]
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn osascript_round_trips_arguments() {
        let args = vec![
            r#"quotes " and backslash \\"#.into(),
            "-leading-dash".into(),
        ];
        let output =
            run_script("function run(argv) { return JSON.stringify(argv); }", &args).unwrap();
        assert!(
            output.success,
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let returned: Vec<String> = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(returned, args);
    }

    #[test]
    fn parses_batched_note_json_without_bodies() {
        let notes: Vec<ScriptNote> = parse(
            &output(
                true,
                r#"[{"id":"x-coredata://1","title":"Plans","folder":"Work","modified":"2026-10-26T14:30:00Z","locked":false},{"id":"x-coredata://2","title":"Secrets","folder":"Work","modified":null,"locked":true}]"#,
                "",
            ),
            Some("Work"),
            "Terminal",
        )
        .unwrap();
        let notes: Vec<Note> = notes.into_iter().map(Into::into).collect();
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes[0].modified,
            Some("2026-10-26T14:30:00Z".parse().unwrap())
        );
        assert!(notes[1].locked);
    }

    #[test]
    fn list_script_batches_properties_and_never_reads_bodies() {
        for property in ["id", "name", "modificationDate", "passwordProtected"] {
            assert!(
                LIST_SCRIPT.contains(&format!("notes.folders.notes.{property}()")),
                "{property}"
            );
            assert!(
                LIST_SCRIPT.contains(&format!("selected.{property}()")),
                "{property}"
            );
        }
        assert!(!LIST_SCRIPT.contains("body"));
    }

    #[test]
    fn folder_selection_detects_ambiguity_before_use() {
        for script in [CREATE_SCRIPT, LIST_SCRIPT] {
            let ambiguity = script.find("if (folders.length > 1)").unwrap();
            let selection = script.find("return folders[0]").unwrap();
            assert!(ambiguity < selection);
        }
    }

    #[test]
    fn maps_automation_and_missing_folder_errors() {
        let denied: Result<Vec<ScriptNote>, _> = parse(
            &output(false, "", "Not authorized to send Apple events. (-1743)"),
            None,
            "Ghostty",
        );
        assert!(matches!(
            denied,
            Err(BackendError::AutomationDenied { terminal, .. }) if terminal == "Ghostty"
        ));

        let missing: Result<Vec<ScriptNote>, _> = parse(
            &output(false, "", "Error: NIB_FOLDER_NOT_FOUND (-2700)"),
            Some("Archive"),
            "Ghostty",
        );
        assert!(matches!(
            missing,
            Err(BackendError::NotFound { kind: "folder", name }) if name == "Archive"
        ));

        let ambiguous: Result<Vec<ScriptNote>, _> = parse(
            &output(
                false,
                "",
                "Error: NIB_FOLDER_AMBIGUOUS:[\"iCloud\",\"On My Mac\"] (-2700)",
            ),
            Some("Work"),
            "Ghostty",
        );
        assert_eq!(
            ambiguous.as_ref().unwrap_err().to_string(),
            "more than one folder is named \"Work\" (matches: account \"iCloud\", account \"On My Mac\")"
        );
        assert!(matches!(
            ambiguous,
            Err(BackendError::Ambiguous { kind: "folder", name, matches })
                if name == "Work"
                    && matches == ["account \"iCloud\"", "account \"On My Mac\""]
        ));
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads and writes real Apple Notes"]
    fn live_create_list_and_delete_tricky_title() {
        if std::env::var("NIB_LIVE_WRITE").as_deref() != Ok("1") {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Apple Notes");
            return;
        }
        let backend = AppleNotes::new();
        let title = format!(r#"nib live {} "quotes" \\ <>&"#, std::process::id());
        let created = backend
            .create(NewNote {
                title: title.clone(),
                folder: None,
            })
            .unwrap();
        let mut cleanup = LiveNoteCleanup {
            id: Some(created.id.clone()),
        };
        let listed = backend.list(NoteQuery {
            folder: created.folder.clone(),
        });
        let deleted = backend.delete(&created.id);
        if deleted.is_ok() {
            cleanup.id = None;
        }
        assert!(deleted.is_ok(), "{deleted:?}");
        let listed = listed.unwrap();
        assert!(
            listed
                .iter()
                .any(|note| note.id == created.id && note.title == title)
        );
    }
}
