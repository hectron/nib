use std::collections::hash_map::RandomState;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub trait Host {
    fn create_edit_file(&mut self, contents: &str) -> io::Result<EditFile> {
        EditFile::create(contents)
    }

    fn edit(&mut self, path: &Path) -> io::Result<Duration>;
    fn ask(&mut self, prompt: &str) -> io::Result<String>;
    fn say(&mut self, message: &str);
    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
    fn journal_files(&mut self) -> io::Result<Vec<JournalFile>>;
}

pub struct System;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalFile {
    pub id: String,
    pub contents: String,
}

pub struct EditFile {
    path: PathBuf,
    remove_on_drop: bool,
}

impl EditFile {
    fn create(contents: &str) -> io::Result<Self> {
        for _ in 0..128 {
            let path = std::env::temp_dir().join(format!("nib-tasks-{}.md", random_token()));
            match secure_new(&path) {
                Ok(mut file) => {
                    if let Err(error) = file.write_all(contents.as_bytes()) {
                        let _ = std::fs::remove_file(&path);
                        return Err(error);
                    }
                    return Ok(Self {
                        path,
                        remove_on_drop: true,
                    });
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "couldn't allocate a unique edit file",
        ))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn read(&self) -> io::Result<String> {
        std::fs::read_to_string(&self.path)
    }

    pub fn preserve(&mut self) {
        self.remove_on_drop = false;
    }

    pub fn remove_on_drop(&mut self) {
        self.remove_on_drop = true;
    }
}

impl Drop for EditFile {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

fn secure_new(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

fn random_token() -> String {
    format!("{:016x}", RandomState::new().hash_one(()))
}

fn backup_name(name: &str) -> String {
    let (stem, extension) = name
        .rsplit_once('.')
        .map_or((name, None), |(stem, extension)| (stem, Some(extension)));
    extension.map_or_else(
        || format!("{stem}-{}", random_token()),
        |extension| format!("{stem}-{}.{extension}", random_token()),
    )
}

fn save_backup_in(dir: &Path, name: &str, contents: &str) -> io::Result<PathBuf> {
    for _ in 0..128 {
        let path = dir.join(backup_name(name));
        match secure_new(&path) {
            Ok(mut file) => {
                if let Err(error) = file.write_all(contents.as_bytes()) {
                    let _ = std::fs::remove_file(&path);
                    return Err(error);
                }
                return Ok(path);
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "couldn't allocate a unique backup file",
    ))
}

fn backups_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME isn't set"))?;
    Ok(PathBuf::from(home).join("Library/Application Support/nib/backups"))
}

fn journal_files_in(dir: &Path) -> io::Result<Vec<JournalFile>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        let Some(id) = path.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        if !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("json"))
            || (!id.contains("-journal-") && !id.ends_with("-journal"))
        {
            continue;
        }
        files.push(JournalFile {
            id: id.to_owned(),
            contents: std::fs::read_to_string(path)?,
        });
    }
    Ok(files)
}

impl Host for System {
    fn edit(&mut self, path: &Path) -> io::Result<Duration> {
        let editor = ["VISUAL", "EDITOR"]
            .into_iter()
            .find_map(|name| std::env::var(name).ok().filter(|v| !v.trim().is_empty()))
            .unwrap_or_else(|| "vi".into());
        let started = Instant::now();
        let status = Command::new("sh")
            .arg("-c")
            .arg(format!("{editor} \"$1\""))
            .arg("nib")
            .arg(path)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("`{editor}` exited with {status}")));
        }
        Ok(started.elapsed())
    }

    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        if !io::stdin().is_terminal() {
            return Ok("n".into());
        }
        eprint!("{prompt}");
        io::stderr().flush()?;
        let mut answer = String::new();
        io::stdin().lock().read_line(&mut answer)?;
        Ok(answer.trim().to_string())
    }

    fn say(&mut self, message: &str) {
        eprintln!("{message}");
    }

    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        let dir = backups_dir()?;
        std::fs::create_dir_all(&dir)?;
        save_backup_in(&dir, name, contents)
    }

    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.save_backup(name, contents)
    }

    fn journal_files(&mut self) -> io::Result<Vec<JournalFile>> {
        journal_files_in(&backups_dir()?)
    }
}

#[cfg(test)]
pub struct Scripted {
    edits: std::collections::VecDeque<Box<dyn FnOnce(String) -> String>>,
    answers: std::collections::VecDeque<String>,
    pub said: Vec<String>,
    pub backups: Vec<(String, String)>,
    pub journals: Vec<(String, String)>,
}

#[cfg(test)]
impl Scripted {
    pub fn new() -> Self {
        Self {
            edits: std::collections::VecDeque::default(),
            answers: std::collections::VecDeque::default(),
            said: Vec::new(),
            backups: Vec::new(),
            journals: Vec::new(),
        }
    }

    pub fn edit(mut self, change: impl FnOnce(String) -> String + 'static) -> Self {
        self.edits.push_back(Box::new(change));
        self
    }

    pub fn answer(mut self, answer: &str) -> Self {
        self.answers.push_back(answer.into());
        self
    }

    pub fn journal(mut self, id: &str, contents: &str) -> Self {
        self.journals.push((format!("{id}.json"), contents.into()));
        self
    }
}

#[cfg(test)]
impl Host for Scripted {
    fn edit(&mut self, path: &Path) -> io::Result<Duration> {
        let change = self
            .edits
            .pop_front()
            .expect("nib opened the editor unexpectedly");
        let text = std::fs::read_to_string(path)?;
        std::fs::write(path, change(text))?;
        Ok(Duration::from_secs(1))
    }

    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        self.said.push(prompt.into());
        Ok(self
            .answers
            .pop_front()
            .expect("nib asked a question unexpectedly"))
    }

    fn say(&mut self, message: &str) {
        self.said.push(message.into());
    }

    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.backups.push((name.into(), contents.into()));
        Ok(PathBuf::from("/backups").join(name))
    }

    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.journals.push((name.into(), contents.into()));
        Ok(PathBuf::from("/backups").join(name))
    }

    fn journal_files(&mut self) -> io::Result<Vec<JournalFile>> {
        Ok(self
            .journals
            .iter()
            .map(|(name, contents)| JournalFile {
                id: name.strip_suffix(".json").unwrap_or(name).to_owned(),
                contents: contents.clone(),
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            for _ in 0..128 {
                let path = std::env::temp_dir().join(format!("nib-host-test-{}", random_token()));
                match std::fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("couldn't create test directory: {error}"),
                }
            }
            panic!("couldn't allocate a unique test directory");
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("test directory should be removable");
        }
    }

    #[test]
    fn edit_files_are_private_unique_and_removed_by_default() {
        let first = EditFile::create("first").unwrap();
        let first_path = first.path().to_owned();
        let second = EditFile::create("second").unwrap();
        assert_ne!(first.path(), second.path());
        assert_eq!(first.read().unwrap(), "first");
        let mode = std::fs::metadata(first.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        drop(first);
        assert!(!first_path.exists());
    }

    #[test]
    fn preserved_edit_files_survive_the_guard() {
        let mut file = EditFile::create("edited").unwrap();
        let path = file.path().to_owned();
        file.preserve();
        drop(file);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "edited");
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn backups_with_the_same_base_name_never_collide() {
        let dir = TestDir::new();
        let first = save_backup_in(&dir.0, "20261026T090000-tasks.json", "first").unwrap();
        let second = save_backup_in(&dir.0, "20261026T090000-tasks.json", "second").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(second).unwrap(), "second");
    }

    #[test]
    fn journal_files_ignore_task_backups() {
        let dir = TestDir::new();
        save_backup_in(&dir.0, "20261026T090000-tasks.json", "[]").unwrap();
        let journal = save_backup_in(&dir.0, "20261026T090000-journal.json", "{}").unwrap();
        let files = journal_files_in(&dir.0).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].contents, "{}");
        assert_eq!(files[0].id, journal.file_stem().unwrap().to_str().unwrap());
    }
}
