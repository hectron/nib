use std::process::Command;

use jiff::Timestamp;
use serde::Deserialize;

use crate::domain::{
    BackendError, NewNote, NewResource, Note, NoteFolder, NoteQuery, Notes, ResourceSelector,
    terminal_name,
};
use crate::infra::completions::CacheFile;

const APP: &str = "Apple Notes";
const FOLDER_AMBIGUOUS: &str = "NIB_FOLDER_AMBIGUOUS:";
const FOLDER_NOT_FOUND: &str = "NIB_FOLDER_NOT_FOUND";
const ACCOUNT_NOT_FOUND: &str = "NIB_ACCOUNT_NOT_FOUND:";
const ACCOUNT_AMBIGUOUS: &str = "NIB_ACCOUNT_AMBIGUOUS:";
const MANAGED_FOLDER_NOT_FOUND: &str = "NIB_MANAGED_FOLDER_NOT_FOUND:";
const MANAGED_FOLDER_AMBIGUOUS: &str = "NIB_MANAGED_FOLDER_AMBIGUOUS:";
const MANAGED_FOLDER_ALREADY_EXISTS: &str = "NIB_MANAGED_FOLDER_ALREADY_EXISTS:";
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
    let folderName = null;
    try {
        folderName = folder.name();
    } catch (error) {
    }
    const modified = created.modificationDate();
    return JSON.stringify({
        folder: folderName,
        note: {
            id: created.id(),
            title: created.name(),
            folder: folderName,
            modified: modified === null ? null : modified.toISOString(),
            locked: created.passwordProtected()
        }
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
    let folders = [];
    if (argv.length === 0) {
        folders = notes.folders.name();
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
        const folderName = folder.name();
        folders = [folderName];
        append(result, folderName, selected.id(), selected.name(), selected.modificationDate(), selected.passwordProtected());
    }
    return JSON.stringify({folders: folders, notes: result});
}
"#;
const FOLDERS_SCRIPT: &str = r#"
function appendFolders(result, seen, folders, accountName, accountId) {
    for (const folder of folders()) {
        const id = folder.id();
        if (seen[id]) {
            continue;
        }
        seen[id] = true;
        result.push({id: id, name: folder.name(), account: accountName, account_id: accountId});
        appendFolders(result, seen, folder.folders, accountName, accountId);
    }
}

function run() {
    const notes = Application("Notes");
    const result = [];
    const seen = Object.create(null);
    for (const account of notes.accounts()) {
        appendFolders(result, seen, account.folders, account.name(), account.id());
    }
    return JSON.stringify(result);
}
"#;
const MANAGE_FOLDERS_SCRIPT: &str = r#"
function accounts(notes) {
    return notes.accounts().map(account => ({
        value: account,
        id: account.id(),
        name: account.name()
    }));
}

function accountNamed(notes, selector) {
    const matches = accounts(notes).filter(account => account.id === selector || account.name === selector);
    if (matches.length === 0) {
        throw new Error("NIB_ACCOUNT_NOT_FOUND:" + JSON.stringify(selector));
    }
    if (matches.length > 1) {
        throw new Error("NIB_ACCOUNT_AMBIGUOUS:" + JSON.stringify(matches.map(account => account.name + " [" + account.id + "]")));
    }
    return matches[0];
}

function appendFolders(result, seen, folders, account) {
    for (const folder of folders()) {
        const id = folder.id();
        if (seen[id]) {
            continue;
        }
        seen[id] = true;
        result.push({
            value: folder,
            id: id,
            name: folder.name(),
            account: account.name,
            account_id: account.id
        });
        appendFolders(result, seen, folder.folders, account);
    }
}

function folders(notes) {
    const result = [];
    const seen = Object.create(null);
    for (const account of accounts(notes)) {
        appendFolders(result, seen, account.value.folders, account);
    }
    return result;
}

function selectedFolder(all, mode, selector, owner) {
    let matches = all.filter(folder => mode === "id" ? folder.id === selector : folder.name === selector);
    if (mode !== "id" && owner !== null) {
        matches = matches.filter(folder => folder.account === owner || folder.account_id === owner);
    }
    if (matches.length === 0) {
        throw new Error("NIB_MANAGED_FOLDER_NOT_FOUND:" + JSON.stringify(selector));
    }
    if (matches.length > 1) {
        throw new Error("NIB_MANAGED_FOLDER_AMBIGUOUS:" + JSON.stringify(matches.map(folder => folder.account + " [" + folder.id + "]")));
    }
    return matches[0];
}

function record(folder) {
    return {
        id: folder.id,
        name: folder.name,
        account: folder.account,
        account_id: folder.account_id
    };
}

function alreadyExists(name, account) {
    throw new Error("NIB_MANAGED_FOLDER_ALREADY_EXISTS:" + JSON.stringify([
        name,
        account.name + " [" + account.id + "]"
    ]));
}

function response(notes, fallback) {
    try {
        const all = folders(notes);
        const changed = all.filter(folder => folder.id === fallback.id)[0];
        return JSON.stringify({
            folder: changed === undefined ? fallback : record(changed),
            folders: all.map(record)
        });
    } catch (error) {
        // The mutation already succeeded. Return the captured resource rather than reporting a
        // refresh failure as if the write itself had failed.
        return JSON.stringify({folder: fallback, folders: null});
    }
}

function run(argv) {
    const notes = Application("Notes");
    if (argv[0] === "create") {
        const account = argv.length === 3
            ? accountNamed(notes, argv[2])
            : {value: notes.defaultAccount(), id: notes.defaultAccount().id(), name: notes.defaultAccount().name()};
        if (folders(notes).some(folder => folder.account_id === account.id && folder.name === argv[1])) {
            alreadyExists(argv[1], account);
        }
        const created = notes.Folder({name: argv[1]});
        account.value.folders.push(created);
        return response(notes, {
            id: created.id(),
            name: argv[1],
            account: account.name,
            account_id: account.id
        });
    }
    const owner = argv.length === 5 ? argv[4] : null;
    const all = folders(notes);
    const selected = selectedFolder(all, argv[1], argv[2], owner);
    if (all.some(folder => folder.id !== selected.id && folder.account_id === selected.account_id && folder.name === argv[3])) {
        alreadyExists(argv[3], {name: selected.account, id: selected.account_id});
    }
    if (selected.name === argv[3]) {
        return response(notes, record(selected));
    }
    selected.value.name = argv[3];
    return response(notes, {
        id: selected.id,
        name: argv[3],
        account: selected.account,
        account_id: selected.account_id
    });
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
    cache: CacheFile,
}

impl AppleNotes {
    pub fn new(cache: CacheFile) -> Self {
        Self {
            terminal: terminal_name(std::env::var("TERM_PROGRAM").ok().as_deref()),
            cache,
        }
    }

    #[cfg(test)]
    fn uncached() -> Self {
        Self::new(CacheFile::new(None, 0))
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
        let created: ScriptCreate = self.execute(CREATE_SCRIPT, &args, note.folder.as_deref())?;
        if let Some(folder) = created.folder {
            self.cache.record_note_folder(folder);
        }
        Ok(created.note.into())
    }

    fn list(&self, query: NoteQuery) -> Result<Vec<Note>, BackendError> {
        let filtered = query.folder.is_some();
        let args: Vec<String> = query.folder.iter().cloned().collect();
        let listed: ScriptList = self.execute(LIST_SCRIPT, &args, query.folder.as_deref())?;
        let folders = listed.folders.into_iter().flatten().collect::<Vec<_>>();
        cache_note_folders(&self.cache, filtered, folders);
        let mut notes: Vec<Note> = listed.notes.into_iter().map(Into::into).collect();
        notes.sort_by(|a: &Note, b: &Note| {
            b.modified.cmp(&a.modified).then_with(|| a.id.cmp(&b.id))
        });
        Ok(notes)
    }

    fn folders(&self) -> Result<Vec<NoteFolder>, BackendError> {
        let mut folders: Vec<ScriptFolder> = self.execute(FOLDERS_SCRIPT, &[], None)?;
        folders.sort_by(|a, b| (&a.name, &a.id).cmp(&(&b.name, &b.id)));
        self.cache
            .update_note_folders(folders.iter().map(|folder| folder.name.clone()).collect());
        Ok(folders.into_iter().map(Into::into).collect())
    }

    fn create_folder(&self, folder: NewResource) -> Result<NoteFolder, BackendError> {
        let account = folder.owner.clone();
        let mut args = vec!["create".into(), folder.name];
        if let Some(account) = folder.owner {
            args.push(account);
        }
        let changed: ScriptFolderMutation =
            self.execute(MANAGE_FOLDERS_SCRIPT, &args, account.as_deref())?;
        Ok(self.finish_folder_mutation(changed))
    }

    fn rename_folder(
        &self,
        folder: ResourceSelector,
        name: String,
    ) -> Result<NoteFolder, BackendError> {
        let (mode, resource, owner) = match folder {
            ResourceSelector::Id(id) => ("id", id, None),
            ResourceSelector::Name { name, owner } => ("name", name, owner),
        };
        let mut args = vec!["rename".into(), mode.into(), resource.clone(), name];
        if let Some(owner) = owner {
            args.push(owner);
        }
        let changed: ScriptFolderMutation =
            self.execute(MANAGE_FOLDERS_SCRIPT, &args, Some(&resource))?;
        Ok(self.finish_folder_mutation(changed))
    }
}

impl AppleNotes {
    fn finish_folder_mutation(&self, changed: ScriptFolderMutation) -> NoteFolder {
        let folder = NoteFolder::from(changed.folder);
        if let Some(folders) = changed.folders {
            self.cache
                .update_note_folders(folders.iter().map(|folder| folder.name.clone()).collect());
        } else {
            self.cache.record_note_folder(folder.name.clone());
        }
        folder
    }
}

fn cache_note_folders(cache: &CacheFile, filtered: bool, folders: Vec<String>) {
    if filtered {
        for folder in folders {
            cache.record_note_folder(folder);
        }
    } else {
        cache.update_note_folders(folders);
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
    } else if let Some(name) = encoded_string(&message, ACCOUNT_NOT_FOUND) {
        BackendError::NotFound {
            kind: "Notes account",
            name,
        }
    } else if let Some(matches) = encoded_strings(&message, ACCOUNT_AMBIGUOUS) {
        BackendError::Ambiguous {
            kind: "Notes account",
            name: folder.unwrap_or_default().into(),
            matches,
        }
    } else if let Some(name) = encoded_string(&message, MANAGED_FOLDER_NOT_FOUND) {
        BackendError::NotFound {
            kind: "folder",
            name,
        }
    } else if let Some(matches) = encoded_strings(&message, MANAGED_FOLDER_AMBIGUOUS) {
        BackendError::Ambiguous {
            kind: "folder",
            name: folder.unwrap_or_default().into(),
            matches,
        }
    } else if let Some([name, owner]) = encoded_strings(&message, MANAGED_FOLDER_ALREADY_EXISTS)
        .and_then(|values| <[String; 2]>::try_from(values).ok())
    {
        BackendError::AlreadyExists {
            kind: "folder",
            name,
            owner,
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

fn encoded_string(message: &str, prefix: &str) -> Option<String> {
    let (_, encoded) = message.split_once(prefix)?;
    let mut deserializer = serde_json::Deserializer::from_str(encoded);
    String::deserialize(&mut deserializer).ok()
}

fn encoded_strings(message: &str, prefix: &str) -> Option<Vec<String>> {
    let (_, encoded) = message.split_once(prefix)?;
    let mut deserializer = serde_json::Deserializer::from_str(encoded);
    Vec::deserialize(&mut deserializer).ok()
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
    folder: Option<String>,
    modified: Option<Timestamp>,
    locked: bool,
}

#[derive(Debug, Deserialize)]
struct ScriptCreate {
    #[serde(default)]
    folder: Option<String>,
    note: ScriptNote,
}

#[derive(Debug, Deserialize)]
struct ScriptList {
    #[serde(default)]
    folders: Vec<Option<String>>,
    notes: Vec<ScriptNote>,
}

#[derive(Debug, Deserialize)]
struct ScriptFolder {
    id: String,
    name: String,
    account: Option<String>,
    account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ScriptFolderMutation {
    folder: ScriptFolder,
    folders: Option<Vec<ScriptFolder>>,
}

impl From<ScriptFolder> for NoteFolder {
    fn from(folder: ScriptFolder) -> Self {
        Self {
            id: folder.id,
            name: folder.name,
            account: folder.account,
            account_id: folder.account_id,
        }
    }
}

impl From<ScriptNote> for Note {
    fn from(note: ScriptNote) -> Self {
        Self {
            id: note.id,
            title: note.title,
            folder: note.folder,
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
                let _ = AppleNotes::uncached().delete(&id);
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
        let listed: ScriptList = parse(
            &output(
                true,
                r#"{"folders":["Work","Archive"],"notes":[{"id":"x-coredata://1","title":"Plans","folder":"Work","modified":"2026-10-26T14:30:00Z","locked":false},{"id":"x-coredata://2","title":"Secrets","folder":"Work","modified":null,"locked":true}]}"#,
                "",
            ),
            Some("Work"),
            "Terminal",
        )
        .unwrap();
        assert_eq!(
            listed.folders,
            [Some("Work".into()), Some("Archive".into())]
        );
        let notes: Vec<Note> = listed.notes.into_iter().map(Into::into).collect();
        assert_eq!(notes.len(), 2);
        assert_eq!(
            notes[0].modified,
            Some("2026-10-26T14:30:00Z".parse().unwrap())
        );
        assert!(notes[1].locked);
    }

    #[test]
    fn parses_created_note_with_its_folder_name() {
        let created: ScriptCreate = parse(
            &output(
                true,
                r#"{"folder":"Notes","note":{"id":"x-coredata://1","title":"Plans","folder":"Notes","modified":"2026-10-26T14:30:00Z","locked":false}}"#,
                "",
            ),
            None,
            "Terminal",
        )
        .unwrap();
        assert_eq!(created.folder.as_deref(), Some("Notes"));
        assert_eq!(created.note.title, "Plans");
        assert!(!CREATE_SCRIPT.contains("notes.folders.name()"));
    }

    #[test]
    fn parses_folders_with_accounts() {
        let folders: Vec<ScriptFolder> = parse(
            &output(
                true,
                r#"[{"id":"x-coredata://folder/1","name":"Work","account":"iCloud","account_id":"account-1"},{"id":"x-coredata://folder/2","name":"Work","account":"Exchange","account_id":"account-2"}]"#,
                "",
            ),
            None,
            "Terminal",
        )
        .unwrap();
        let folders = folders
            .into_iter()
            .map(NoteFolder::from)
            .collect::<Vec<_>>();
        assert_eq!(folders[0].account.as_deref(), Some("iCloud"));
        assert_eq!(folders[0].account_id.as_deref(), Some("account-1"));
        assert_eq!(folders[1].account.as_deref(), Some("Exchange"));
    }

    #[test]
    fn folder_discovery_recurses_from_each_account() {
        assert!(FOLDERS_SCRIPT.contains("for (const account of notes.accounts())"));
        assert!(
            FOLDERS_SCRIPT
                .contains("appendFolders(result, seen, folder.folders, accountName, accountId)")
        );
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads your real Notes folders; run with --ignored from a terminal that has access"]
    fn lists_real_folders() {
        AppleNotes::uncached().folders().unwrap();
    }

    #[test]
    fn folder_cache_fields_allow_missing_and_null_values() {
        let created: ScriptCreate = parse(
            &output(
                true,
                r#"{"note":{"id":"x-coredata://1","title":"Plans","folder":null,"modified":null,"locked":false}}"#,
                "",
            ),
            None,
            "Terminal",
        )
        .unwrap();
        assert!(created.folder.is_none());
        assert!(created.note.folder.is_none());

        let listed: ScriptList = parse(
            &output(true, r#"{"folders":["Work",null],"notes":[]}"#, ""),
            None,
            "Terminal",
        )
        .unwrap();
        assert_eq!(listed.folders, [Some("Work".into()), None]);
    }

    #[test]
    fn filtered_folder_results_merge_into_the_cache() {
        let directory =
            std::env::temp_dir().join(format!("nib-notes-filtered-cache-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&directory);
        let cache = CacheFile::new(Some(&directory), 1_000);
        assert_eq!(
            cache.values(crate::cli::CompletionKind::NoteFolders),
            Vec::<String>::new()
        );
        cache.update_note_folders(vec!["Archive".into(), "Personal".into()]);
        cache_note_folders(&cache, true, vec!["Work".into()]);
        assert_eq!(
            cache.values(crate::cli::CompletionKind::NoteFolders),
            ["Archive", "Personal", "Work"]
        );
        cache_note_folders(&cache, false, vec!["Work".into()]);
        assert_eq!(
            cache.values(crate::cli::CompletionKind::NoteFolders),
            ["Work"]
        );
        std::fs::remove_dir_all(directory).unwrap();
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
        let denied: Result<ScriptList, _> = parse(
            &output(false, "", "Not authorized to send Apple events. (-1743)"),
            None,
            "Ghostty",
        );
        assert!(matches!(
            denied,
            Err(BackendError::AutomationDenied { terminal, .. }) if terminal == "Ghostty"
        ));

        let missing: Result<ScriptList, _> = parse(
            &output(false, "", "Error: NIB_FOLDER_NOT_FOUND (-2700)"),
            Some("Archive"),
            "Ghostty",
        );
        assert!(matches!(
            missing,
            Err(BackendError::NotFound { kind: "folder", name }) if name == "Archive"
        ));

        let ambiguous: Result<ScriptList, _> = parse(
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
    fn managed_folder_errors_keep_the_requested_subject() {
        let account: Result<ScriptFolderMutation, _> = parse(
            &output(
                false,
                "",
                "Error: NIB_ACCOUNT_AMBIGUOUS:[\"iCloud [a]\",\"iCloud [b]\"] (-2700)",
            ),
            Some("iCloud"),
            "Ghostty",
        );
        assert!(matches!(
            account,
            Err(BackendError::Ambiguous {
                kind: "Notes account",
                name,
                ..
            }) if name == "iCloud"
        ));

        let folder: Result<ScriptFolderMutation, _> = parse(
            &output(
                false,
                "",
                "Error: NIB_MANAGED_FOLDER_NOT_FOUND:\"Archive\" (-2700)",
            ),
            Some("Archive"),
            "Ghostty",
        );
        assert!(matches!(
            folder,
            Err(BackendError::NotFound { kind: "folder", name }) if name == "Archive"
        ));

        let collision: Result<ScriptFolderMutation, _> = parse(
            &output(
                false,
                "",
                "Error: NIB_MANAGED_FOLDER_ALREADY_EXISTS:[\"Work\",\"iCloud [account-1]\"] (-2700)",
            ),
            Some("Personal"),
            "Ghostty",
        );
        assert!(matches!(
            collision,
            Err(BackendError::AlreadyExists {
                kind: "folder",
                name,
                owner,
            }) if name == "Work" && owner == "iCloud [account-1]"
        ));
    }

    #[test]
    fn folder_mutation_falls_back_when_post_write_refresh_is_unavailable() {
        let changed: ScriptFolderMutation = parse(
            &output(
                true,
                r#"{"folder":{"id":"folder-1","name":"Projects","account":"iCloud","account_id":"account-1"},"folders":null}"#,
                "",
            ),
            None,
            "Terminal",
        )
        .unwrap();
        let folder = AppleNotes::uncached().finish_folder_mutation(changed);
        assert_eq!(folder.id, "folder-1");
        assert_eq!(folder.name, "Projects");
        assert_eq!(folder.account_id.as_deref(), Some("account-1"));
    }

    #[test]
    fn managed_folder_script_checks_collisions_and_preserves_write_errors() {
        let rename = MANAGE_FOLDERS_SCRIPT
            .rfind("const selected = selectedFolder")
            .unwrap();
        let collision = MANAGE_FOLDERS_SCRIPT[rename..]
            .find("if (all.some")
            .map(|offset| rename + offset)
            .unwrap();
        let no_op = MANAGE_FOLDERS_SCRIPT[rename..]
            .find("if (selected.name === argv[3])")
            .map(|offset| rename + offset)
            .unwrap();
        let write = MANAGE_FOLDERS_SCRIPT[rename..]
            .find("selected.value.name = argv[3]")
            .map(|offset| rename + offset)
            .unwrap();
        let response = MANAGE_FOLDERS_SCRIPT[write..]
            .find("return response(notes")
            .map(|offset| write + offset)
            .unwrap();
        assert!(collision < no_op);
        assert!(no_op < write);
        assert!(write < response);
    }

    #[test]
    #[cfg(target_os = "macos")]
    #[ignore = "reads and writes real Apple Notes"]
    fn live_create_list_and_delete_tricky_title() {
        if std::env::var("NIB_LIVE_WRITE").as_deref() != Ok("1") {
            eprintln!("skipped: set NIB_LIVE_WRITE=1 to write to your real Apple Notes");
            return;
        }
        let backend = AppleNotes::uncached();
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
