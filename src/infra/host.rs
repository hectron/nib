use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

use jiff::Timestamp;
use rustix::fs::FlockOperation;

const STALE_TEMPORARY_AFTER: Duration = Duration::from_secs(24 * 60 * 60);

pub trait JournalLock {}

pub trait Host {
    fn create_edit_file(&mut self, contents: &str) -> io::Result<EditFile> {
        EditFile::create(contents)
    }

    fn edit(&mut self, path: &Path) -> io::Result<Duration>;
    fn ask(&mut self, prompt: &str) -> io::Result<String>;
    fn say(&mut self, message: &str);
    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
    fn replace_journal(&mut self, id: &str, contents: &str) -> io::Result<PathBuf>;
    fn remove_journal(&mut self, id: &str) -> io::Result<()>;
    fn journal_files(&mut self, now: Timestamp) -> io::Result<Vec<JournalFile>>;
    fn journal_files_without_cleanup(&mut self) -> io::Result<Vec<JournalFile>>;
    fn lock_journal(&mut self) -> io::Result<Box<dyn JournalLock>>;
    fn try_lock_journal(&mut self) -> io::Result<Option<Box<dyn JournalLock>>>;
}

pub struct System;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalFile {
    pub id: String,
    pub path: PathBuf,
    pub written: u128,
    pub written_at: Option<Timestamp>,
    pub contents: Result<String, String>,
}

struct PublishedJournal {
    path: PathBuf,
    warning: Option<String>,
}

impl JournalLock for File {}
impl JournalLock for () {}

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

fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

fn open_journal_lock(dir: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(dir.join("journal.lock"))
}

fn try_lock_journal_in(dir: &Path) -> io::Result<Option<Box<dyn JournalLock>>> {
    if !dir.is_dir() {
        return Ok(None);
    }
    let file = open_journal_lock(dir)?;
    match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(Some(Box::new(file))),
        Err(rustix::io::Errno::WOULDBLOCK) => Ok(None),
        Err(error) => Err(error.into()),
    }
}

fn lock_exclusive(file: &File, waiting: impl FnOnce()) -> io::Result<()> {
    match rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(()),
        Err(rustix::io::Errno::WOULDBLOCK) => {
            waiting();
            rustix::fs::flock(file, FlockOperation::LockExclusive).map_err(Into::into)
        }
        Err(error) => Err(error.into()),
    }
}

pub(crate) fn atomic_write(path: &Path, contents: &str) -> io::Result<AtomicWriteOutcome> {
    atomic_write_with(path, contents, || Ok(()))
}

#[derive(Debug)]
pub(crate) struct AtomicWriteOutcome {
    directory_sync_error: Option<(PathBuf, io::Error)>,
}

impl AtomicWriteOutcome {
    pub(crate) fn directory_sync_error(self) -> Option<(PathBuf, io::Error)> {
        self.directory_sync_error
    }
}

fn atomic_write_with(
    path: &Path,
    contents: &str,
    before_rename: impl FnOnce() -> io::Result<()>,
) -> io::Result<AtomicWriteOutcome> {
    atomic_write_with_sync(path, contents, before_rename, sync_directory)
}

fn atomic_write_with_sync(
    path: &Path,
    contents: &str,
    before_rename: impl FnOnce() -> io::Result<()>,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<AtomicWriteOutcome> {
    let target = resolved_target(path)?;
    let directory = target
        .parent()
        .ok_or_else(|| io::Error::other("configuration path has no parent directory"))?;
    let permissions = match std::fs::metadata(&target) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => None,
        Err(error) => return Err(error),
    };
    let file_name = target
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    remove_stale_temporary_files(directory, file_name, SystemTime::now());

    for _ in 0..128 {
        let temporary = directory.join(format!(".{file_name}.nib-{}.tmp", random_token()));
        let opened = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(if permissions.is_some() { 0o600 } else { 0o666 })
            .open(&temporary);
        let mut file = match opened {
            Ok(file) => file,
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        };
        let mut renamed = false;
        let result = (|| {
            file.write_all(contents.as_bytes())?;
            if let Some(permissions) = &permissions {
                file.set_permissions(permissions.clone())?;
            }
            file.sync_all()?;
            before_rename()?;
            std::fs::rename(&temporary, &target)?;
            renamed = true;
            Ok(AtomicWriteOutcome {
                directory_sync_error: sync_directory(directory)
                    .err()
                    .map(|error| (directory.to_owned(), error)),
            })
        })();
        if result.is_err() && !renamed {
            let _ = std::fs::remove_file(&temporary);
        }
        return result;
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "couldn't allocate a unique configuration temporary file",
    ))
}

fn resolved_target(path: &Path) -> io::Result<PathBuf> {
    match std::fs::canonicalize(path) {
        Ok(target) => Ok(target),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            match std::fs::symlink_metadata(path) {
                Err(metadata_error) if metadata_error.kind() == io::ErrorKind::NotFound => {
                    Ok(path.to_owned())
                }
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    }
}

fn remove_stale_temporary_files(directory: &Path, file_name: &str, now: SystemTime) {
    let prefix = format!(".{file_name}.nib-");
    let Ok(entries) = std::fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix)
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "tmp")
        {
            let old = entry
                .metadata()
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_TEMPORARY_AFTER);
            if old {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub(crate) struct ConfigLock {
    _file: File,
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ConfigLockError {
    #[error("configuration {} is already locked", path.display())]
    Held { path: PathBuf },
    #[error(transparent)]
    Target(#[from] io::Error),
    #[error("couldn't lock configuration with {}: {source}", path.display())]
    Lock { path: PathBuf, source: io::Error },
}

pub(crate) fn legacy_config_lock_path(path: &Path) -> io::Result<PathBuf> {
    let target = resolved_target(path)?;
    let file_name = target
        .file_name()
        .map_or_else(|| OsString::from("config"), std::ffi::OsStr::to_os_string);
    let mut lock_name = file_name;
    lock_name.push(".lock");
    Ok(target.with_file_name(lock_name))
}

fn config_lock_target(path: &Path) -> io::Result<PathBuf> {
    let target = resolved_target(path)?;
    if target.exists() {
        return Ok(target);
    }
    let canonical_parent = target
        .parent()
        .zip(target.file_name())
        .and_then(|(parent, name)| Some(std::fs::canonicalize(parent).ok()?.join(name)));
    Ok(canonical_parent.unwrap_or(target))
}

fn config_lock_name(target: &Path) -> String {
    format!(
        "config-{:016x}.lock",
        fnv1a_64(target.as_os_str().as_bytes())
    )
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub(crate) fn config_lock(path: &Path, dir: &Path) -> Result<ConfigLock, ConfigLockError> {
    let target = config_lock_target(path)?;
    std::fs::create_dir_all(dir).map_err(|source| ConfigLockError::Lock {
        path: dir.to_owned(),
        source,
    })?;
    let lock_path = dir.join(config_lock_name(&target));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(&lock_path)
        .map_err(|source| ConfigLockError::Lock {
            path: lock_path.clone(),
            source,
        })?;
    match rustix::fs::flock(&file, FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(ConfigLock { _file: file }),
        Err(rustix::io::Errno::WOULDBLOCK) => Err(ConfigLockError::Held { path: target }),
        Err(error) => Err(ConfigLockError::Lock {
            path: lock_path,
            source: error.into(),
        }),
    }
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

fn save_journal_in(dir: &Path, name: &str, contents: &str) -> io::Result<PublishedJournal> {
    for _ in 0..128 {
        let path = dir.join(backup_name(name));
        let temporary = path.with_extension(format!("json.{}.tmp", random_token()));
        let result = (|| {
            let mut file = secure_new(&temporary)?;
            file.write_all(contents.as_bytes())?;
            file.sync_all()?;
            std::fs::hard_link(&temporary, &path)
        })();
        match result {
            Ok(()) => {
                let warning = sync_directory(dir).err().map(|error| {
                    format!(
                        "journal {} was written, but its folder couldn't be synced: {error}",
                        path.display()
                    )
                });
                let _ = std::fs::remove_file(&temporary);
                return Ok(PublishedJournal { path, warning });
            }
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = std::fs::remove_file(&temporary);
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temporary);
                return Err(io::Error::new(
                    error.kind(),
                    format!("couldn't write journal {}: {error}", path.display()),
                ));
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "couldn't allocate a unique journal file",
    ))
}

fn replace_journal_in(dir: &Path, id: &str, contents: &str) -> io::Result<PublishedJournal> {
    let path = dir.join(format!("{id}.json"));
    let temporary = path.with_extension(format!("json.{}.tmp", random_token()));
    let result: io::Result<()> = (|| {
        let mut file = secure_new(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    match result {
        Ok(()) => {
            let warning = sync_directory(dir).err().map(|error| {
                format!(
                    "journal {} was updated, but its folder couldn't be synced: {error}",
                    path.display()
                )
            });
            Ok(PublishedJournal { path, warning })
        }
        Err(error) => {
            let _ = std::fs::remove_file(&temporary);
            Err(io::Error::new(
                error.kind(),
                format!("couldn't update journal {}: {error}", path.display()),
            ))
        }
    }
}

fn remove_journal_in(
    dir: &Path,
    id: &str,
    sync_dir: impl FnOnce(&Path) -> io::Result<()>,
) -> io::Result<Option<String>> {
    let path = dir.join(format!("{id}.json"));
    std::fs::remove_file(&path)?;
    Ok(sync_dir(dir).err().map(|error| {
        format!(
            "journal {} was removed, but its folder couldn't be synced: {error}",
            path.display()
        )
    }))
}

fn backups_dir() -> io::Result<PathBuf> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME isn't set"))?;
    Ok(PathBuf::from(home).join("Library/Application Support/nib/backups"))
}

fn journal_files_in(dir: &Path, now: Timestamp) -> io::Result<Vec<JournalFile>> {
    list_journal_files(dir, Some(now))
}

fn list_journal_files(dir: &Path, cleanup_at: Option<Timestamp>) -> io::Result<Vec<JournalFile>> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut files = Vec::new();
    for entry in entries {
        let entry = entry?;
        let path = entry.path();
        if cleanup_at.is_some_and(|now| is_stale_journal_temporary(&path, now)) {
            let _ = std::fs::remove_file(&path);
            continue;
        }
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
        let contents = std::fs::read_to_string(&path).map_err(|error| error.to_string());
        let written_at = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .ok()
            .and_then(system_time_to_timestamp);
        let written = written_at.map_or(0, timestamp_key);
        files.push(JournalFile {
            id: id.to_owned(),
            path,
            written,
            written_at,
            contents,
        });
    }
    Ok(files)
}

fn is_stale_journal_temporary(path: &Path, now: Timestamp) -> bool {
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return false;
    };
    if !name.contains("-journal")
        || !path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("tmp"))
    {
        return false;
    }
    std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
        .and_then(system_time_to_timestamp)
        .is_some_and(|modified| now.as_second().saturating_sub(modified.as_second()) > 86_400)
}

fn system_time_to_timestamp(time: SystemTime) -> Option<Timestamp> {
    let duration = time.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    let seconds = i64::try_from(duration.as_secs()).ok()?;
    Timestamp::new(seconds, duration.subsec_nanos().try_into().ok()?).ok()
}

fn timestamp_key(timestamp: Timestamp) -> u128 {
    u128::try_from(timestamp.as_second()).unwrap_or_default() * 1_000_000_000
        + u128::try_from(timestamp.subsec_nanosecond()).unwrap_or_default()
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
        let dir = backups_dir()?;
        std::fs::create_dir_all(&dir)?;
        let published = save_journal_in(&dir, name, contents)?;
        if let Some(warning) = published.warning {
            self.say(&format!("warning: {warning}"));
        }
        Ok(published.path)
    }

    fn replace_journal(&mut self, id: &str, contents: &str) -> io::Result<PathBuf> {
        let published = replace_journal_in(&backups_dir()?, id, contents)?;
        if let Some(warning) = published.warning {
            self.say(&format!("warning: {warning}"));
        }
        Ok(published.path)
    }

    fn remove_journal(&mut self, id: &str) -> io::Result<()> {
        let dir = backups_dir()?;
        if let Some(warning) = remove_journal_in(&dir, id, sync_directory)? {
            self.say(&format!("warning: {warning}"));
        }
        Ok(())
    }

    fn journal_files(&mut self, now: Timestamp) -> io::Result<Vec<JournalFile>> {
        journal_files_in(&backups_dir()?, now)
    }

    fn journal_files_without_cleanup(&mut self) -> io::Result<Vec<JournalFile>> {
        list_journal_files(&backups_dir()?, None)
    }

    fn lock_journal(&mut self) -> io::Result<Box<dyn JournalLock>> {
        let dir = backups_dir()?;
        std::fs::create_dir_all(&dir)?;
        let file = open_journal_lock(&dir)?;
        lock_exclusive(&file, || {
            self.say("Waiting for another nib command to finish…");
        })?;
        Ok(Box::new(file))
    }

    fn try_lock_journal(&mut self) -> io::Result<Option<Box<dyn JournalLock>>> {
        try_lock_journal_in(&backups_dir()?)
    }
}

pub struct Memory {
    system: System,
    backups: Vec<(String, String)>,
    journals: Vec<(String, String, u128)>,
    sequence: u128,
}

impl Memory {
    pub fn new() -> Self {
        Self {
            system: System,
            backups: Vec::new(),
            journals: Vec::new(),
            sequence: 0,
        }
    }
}

impl Host for Memory {
    fn edit(&mut self, path: &Path) -> io::Result<Duration> {
        self.system.edit(path)
    }

    fn ask(&mut self, prompt: &str) -> io::Result<String> {
        self.system.ask(prompt)
    }

    fn say(&mut self, message: &str) {
        self.system.say(message);
    }

    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.backups.push((name.into(), contents.into()));
        Ok(PathBuf::from("demo-backups").join(name))
    }

    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.sequence += 1;
        let id = backup_name(name);
        self.journals
            .push((id.clone(), contents.into(), self.sequence));
        Ok(PathBuf::from("demo-backups").join(id))
    }

    fn replace_journal(&mut self, id: &str, contents: &str) -> io::Result<PathBuf> {
        self.sequence += 1;
        let entry = self
            .journals
            .iter_mut()
            .find(|(name, _, _)| name.strip_suffix(".json").unwrap_or(name) == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "journal entry not found"))?;
        entry.1 = contents.into();
        entry.2 = self.sequence;
        Ok(PathBuf::from("demo-backups").join(&entry.0))
    }

    fn remove_journal(&mut self, id: &str) -> io::Result<()> {
        let index = self
            .journals
            .iter()
            .position(|(name, _, _)| name.strip_suffix(".json").unwrap_or(name) == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "journal entry not found"))?;
        self.journals.remove(index);
        Ok(())
    }

    fn journal_files(&mut self, _now: Timestamp) -> io::Result<Vec<JournalFile>> {
        self.journal_files_without_cleanup()
    }

    fn journal_files_without_cleanup(&mut self) -> io::Result<Vec<JournalFile>> {
        Ok(self
            .journals
            .iter()
            .map(|(name, contents, written)| JournalFile {
                id: name.strip_suffix(".json").unwrap_or(name).to_owned(),
                path: PathBuf::from("demo-backups").join(name),
                written: *written,
                written_at: None,
                contents: Ok(contents.clone()),
            })
            .collect())
    }

    fn lock_journal(&mut self) -> io::Result<Box<dyn JournalLock>> {
        Ok(Box::new(()))
    }

    fn try_lock_journal(&mut self) -> io::Result<Option<Box<dyn JournalLock>>> {
        Ok(Some(Box::new(())))
    }
}

#[cfg(test)]
pub struct Scripted {
    edits: std::collections::VecDeque<Box<dyn FnOnce(String) -> String>>,
    answers: std::collections::VecDeque<String>,
    pub said: Vec<String>,
    pub backups: Vec<(String, String)>,
    pub journals: Vec<(String, String)>,
    unreadable_journals: Vec<(String, String)>,
    sequence: u128,
    fail_replace_once: bool,
    fail_remove_once: bool,
    journal_replacement_on_answer: Option<(String, String)>,
    journal_removal_on_answer: Option<String>,
    journal_unreadable_on_answer: Option<(String, String)>,
    active_journal_locks: std::rc::Rc<std::cell::Cell<usize>>,
    pub journal_lock_count: usize,
    pub journal_listings_with_cleanup: usize,
    pub journal_listings_without_cleanup: usize,
}

#[cfg(test)]
struct ScriptedJournalLock {
    active: std::rc::Rc<std::cell::Cell<usize>>,
}

#[cfg(test)]
impl JournalLock for ScriptedJournalLock {}

#[cfg(test)]
impl Drop for ScriptedJournalLock {
    fn drop(&mut self) {
        self.active.set(self.active.get() - 1);
    }
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
            unreadable_journals: Vec::new(),
            sequence: 0,
            fail_replace_once: false,
            fail_remove_once: false,
            journal_replacement_on_answer: None,
            journal_removal_on_answer: None,
            journal_unreadable_on_answer: None,
            active_journal_locks: std::rc::Rc::new(std::cell::Cell::new(0)),
            journal_lock_count: 0,
            journal_listings_with_cleanup: 0,
            journal_listings_without_cleanup: 0,
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

    pub fn unreadable_journal(mut self, id: &str, error: &str) -> Self {
        self.unreadable_journals
            .push((format!("{id}.json"), error.into()));
        self
    }

    pub fn replace_journal_on_answer(mut self, id: &str, contents: &str) -> Self {
        self.journal_replacement_on_answer = Some((id.into(), contents.into()));
        self
    }

    pub fn remove_journal_on_answer(mut self, id: &str) -> Self {
        self.journal_removal_on_answer = Some(id.into());
        self
    }

    pub fn unreadable_journal_on_answer(mut self, id: &str, error: &str) -> Self {
        self.journal_unreadable_on_answer = Some((id.into(), error.into()));
        self
    }

    pub fn journal_lock_state(&self) -> std::rc::Rc<std::cell::Cell<usize>> {
        self.active_journal_locks.clone()
    }

    pub fn fail_next_journal_replace(&mut self) {
        self.fail_replace_once = true;
    }

    pub fn fail_next_journal_remove(&mut self) {
        self.fail_remove_once = true;
    }

    fn listed_journals(&self) -> Vec<JournalFile> {
        let readable = self
            .journals
            .iter()
            .enumerate()
            .filter(|(_, (name, _))| {
                !self
                    .unreadable_journals
                    .iter()
                    .any(|(unreadable, _)| unreadable == name)
            })
            .map(|(index, (name, contents))| JournalFile {
                id: name.strip_suffix(".json").unwrap_or(name).to_owned(),
                path: PathBuf::from("/backups").join(name),
                written: u128::try_from(index).unwrap_or_default(),
                written_at: None,
                contents: Ok(contents.clone()),
            });
        let unreadable =
            self.unreadable_journals
                .iter()
                .enumerate()
                .map(|(index, (name, error))| JournalFile {
                    id: name.strip_suffix(".json").unwrap_or(name).to_owned(),
                    path: PathBuf::from("/backups").join(name),
                    written: u128::try_from(self.journals.len() + index).unwrap_or_default(),
                    written_at: None,
                    contents: Err(error.clone()),
                });
        readable.chain(unreadable).collect()
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
        assert_eq!(
            self.active_journal_locks.get(),
            0,
            "the journal lock must not be held while waiting for an answer"
        );
        self.said.push(prompt.into());
        let answer = self
            .answers
            .pop_front()
            .expect("nib asked a question unexpectedly");
        if let Some((id, contents)) = self.journal_replacement_on_answer.take() {
            let entry = self
                .journals
                .iter_mut()
                .find(|(name, _)| name.strip_suffix(".json").unwrap_or(name) == id)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "journal entry not found")
                })?;
            entry.1 = contents;
        }
        if let Some((id, error)) = self.journal_unreadable_on_answer.take() {
            let (name, _) = self
                .journals
                .iter()
                .find(|(name, _)| name.strip_suffix(".json").unwrap_or(name) == id)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "journal entry not found")
                })?;
            self.unreadable_journals.push((name.clone(), error));
        }
        if let Some(id) = self.journal_removal_on_answer.take() {
            let index = self
                .journals
                .iter()
                .position(|(name, _)| name.strip_suffix(".json").unwrap_or(name) == id)
                .ok_or_else(|| {
                    io::Error::new(io::ErrorKind::NotFound, "journal entry not found")
                })?;
            self.journals.remove(index);
        }
        Ok(answer)
    }

    fn say(&mut self, message: &str) {
        self.said.push(message.into());
    }

    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.backups.push((name.into(), contents.into()));
        Ok(PathBuf::from("/backups").join(name))
    }

    fn save_journal(&mut self, name: &str, contents: &str) -> io::Result<PathBuf> {
        self.sequence += 1;
        let saved_name = if self.journals.iter().any(|(saved, _)| saved == name) {
            let (stem, extension) = name.rsplit_once('.').unwrap_or((name, "json"));
            format!("{stem}-test{}.{}", self.sequence, extension)
        } else {
            name.into()
        };
        self.journals.push((saved_name.clone(), contents.into()));
        Ok(PathBuf::from("/backups").join(saved_name))
    }

    fn replace_journal(&mut self, id: &str, contents: &str) -> io::Result<PathBuf> {
        if self.fail_replace_once {
            self.fail_replace_once = false;
            return Err(io::Error::other("injected journal save failure"));
        }
        self.sequence += 1;
        let entry = self
            .journals
            .iter_mut()
            .find(|(name, _)| name.strip_suffix(".json").unwrap_or(name) == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "journal entry not found"))?;
        entry.1 = contents.into();
        Ok(PathBuf::from("/backups").join(&entry.0))
    }

    fn remove_journal(&mut self, id: &str) -> io::Result<()> {
        if self.fail_remove_once {
            self.fail_remove_once = false;
            return Err(io::Error::other("injected journal removal failure"));
        }
        let index = self
            .journals
            .iter()
            .position(|(name, _)| name.strip_suffix(".json").unwrap_or(name) == id)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "journal entry not found"))?;
        self.journals.remove(index);
        Ok(())
    }

    fn journal_files(&mut self, _now: Timestamp) -> io::Result<Vec<JournalFile>> {
        self.journal_listings_with_cleanup += 1;
        Ok(self.listed_journals())
    }

    fn journal_files_without_cleanup(&mut self) -> io::Result<Vec<JournalFile>> {
        self.journal_listings_without_cleanup += 1;
        Ok(self.listed_journals())
    }

    fn lock_journal(&mut self) -> io::Result<Box<dyn JournalLock>> {
        assert_eq!(
            self.active_journal_locks.get(),
            0,
            "journal lock is already held; a second lock would deadlock"
        );
        self.journal_lock_count += 1;
        self.active_journal_locks
            .set(self.active_journal_locks.get() + 1);
        Ok(Box::new(ScriptedJournalLock {
            active: self.active_journal_locks.clone(),
        }))
    }

    fn try_lock_journal(&mut self) -> io::Result<Option<Box<dyn JournalLock>>> {
        if self.active_journal_locks.get() > 0 {
            return Ok(None);
        }
        self.lock_journal().map(Some)
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::mpsc;
    use std::thread;

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
    fn exclusive_lock_waits_for_the_current_holder() {
        let dir = TestDir::new();
        let path = dir.0.join("journal.lock");
        let open = || {
            OpenOptions::new()
                .read(true)
                .write(true)
                .create(true)
                .truncate(false)
                .open(&path)
                .unwrap()
        };
        let first = open();
        let second = open();
        lock_exclusive(&first, || panic!("the first lock should be immediate")).unwrap();

        let (waiting_tx, waiting_rx) = mpsc::channel();
        let worker =
            thread::spawn(move || lock_exclusive(&second, || waiting_tx.send(()).unwrap()));
        waiting_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("the second lock should report that it is waiting");
        rustix::fs::flock(&first, FlockOperation::Unlock).unwrap();
        worker.join().unwrap().unwrap();
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
    fn atomic_writes_preserve_symlinks_and_target_modes() {
        let dir = TestDir::new();
        let target = dir.0.join("target.toml");
        let link = dir.0.join("config.toml");
        std::fs::write(&target, "old").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();

        atomic_write(&link, "new").unwrap();

        assert_eq!(std::fs::read_link(&link).unwrap(), target);
        assert_eq!(std::fs::read_to_string(&link).unwrap(), "new");
        let mode = std::fs::metadata(&link).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn interrupted_atomic_write_leaves_the_original_untouched() {
        let dir = TestDir::new();
        let path = dir.0.join("config.toml");
        std::fs::write(&path, "original").unwrap();

        let error = atomic_write_with(&path, "replacement", || {
            Err(io::Error::other("simulated interruption"))
        })
        .unwrap_err();

        assert_eq!(error.to_string(), "simulated interruption");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "original");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn directory_sync_failure_after_rename_is_a_warning() {
        let dir = TestDir::new();
        let path = dir.0.join("config.toml");
        std::fs::write(&path, "original").unwrap();

        let outcome = atomic_write_with_sync(
            &path,
            "replacement",
            || Ok(()),
            |_| Err(io::Error::other("simulated directory sync failure")),
        )
        .unwrap();

        let (_, error) = outcome.directory_sync_error().unwrap();
        assert_eq!(error.to_string(), "simulated directory sync failure");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "replacement");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn next_atomic_write_removes_only_old_matching_temporary_files() {
        let dir = TestDir::new();
        let path = dir.0.join("config.toml");
        let old = dir.0.join(".config.toml.nib-old.tmp");
        let recent = dir.0.join(".config.toml.nib-recent.tmp");
        let unrelated = dir.0.join(".other.toml.nib-old.tmp");
        let old_file = File::create(&old).unwrap();
        File::create(&recent).unwrap();
        File::create(&unrelated).unwrap();
        old_file
            .set_times(std::fs::FileTimes::new().set_modified(
                SystemTime::now() - STALE_TEMPORARY_AFTER - Duration::from_secs(60 * 60),
            ))
            .unwrap();

        atomic_write(&path, "new").unwrap();

        assert!(!old.exists());
        assert!(recent.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn stale_temporary_entries_that_cannot_be_removed_do_not_fail_a_write() {
        let dir = TestDir::new();
        let path = dir.0.join("config.toml");
        std::fs::write(&path, "old").unwrap();
        let stale = dir.0.join(".config.toml.nib-stuck.tmp");
        std::fs::create_dir(&stale).unwrap();
        File::open(&stale)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(
                SystemTime::now() - STALE_TEMPORARY_AFTER - Duration::from_secs(60 * 60),
            ))
            .unwrap();

        atomic_write(&path, "new").unwrap();

        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        assert!(stale.is_dir());
    }

    #[test]
    fn atomic_writes_succeed_in_directories_that_cannot_be_listed() {
        let dir = TestDir::new();
        let path = dir.0.join("config.toml");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o300)).unwrap();

        let result = atomic_write(&path, "new");

        std::fs::set_permissions(&dir.0, std::fs::Permissions::from_mode(0o700)).unwrap();
        result.unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
    }

    #[test]
    fn new_atomic_files_use_the_process_creation_mode() {
        let dir = TestDir::new();
        let public = dir.0.join("public");
        let private = dir.0.join("private");
        std::fs::create_dir(&public).unwrap();
        std::fs::create_dir(&private).unwrap();
        std::fs::set_permissions(&public, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o700)).unwrap();

        let public_file = public.join("config.toml");
        let private_file = private.join("config.toml");
        atomic_write(&public_file, "public").unwrap();
        atomic_write(&private_file, "private").unwrap();

        let public_mode = std::fs::metadata(public_file).unwrap().permissions().mode() & 0o777;
        let private_mode = std::fs::metadata(private_file)
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(public_mode, private_mode);
        assert_eq!(public_mode & 0o111, 0);
        assert_ne!(public_mode & 0o200, 0);
    }

    #[test]
    fn journal_files_ignore_task_backups() {
        let dir = TestDir::new();
        save_backup_in(&dir.0, "20261026T090000-tasks.json", "[]").unwrap();
        let journal = save_journal_in(&dir.0, "20261026T090000-journal.json", "{}").unwrap();
        let mut temporary = secure_new(
            &dir.0
                .join("20261026T090000-journal-deadbeef.json.feedface.tmp"),
        )
        .unwrap();
        temporary.write_all(b"partial").unwrap();
        let now: Timestamp = "2026-10-26T14:00:00Z".parse().unwrap();
        let files = journal_files_in(&dir.0, now).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].contents.as_deref(), Ok("{}"));
        assert_eq!(
            files[0].id,
            journal.path.file_stem().unwrap().to_str().unwrap()
        );
        assert!(temporary.metadata().is_ok());
    }

    #[test]
    fn journal_files_remove_temporary_files_older_than_a_day() {
        let dir = TestDir::new();
        let path = dir
            .0
            .join("20261024T090000-journal-deadbeef.json.feedface.tmp");
        secure_new(&path).unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();

        let now: Timestamp = "2026-06-26T00:00:01Z".parse().unwrap();
        journal_files_in(&dir.0, now).unwrap();

        assert!(!path.exists());
    }

    #[test]
    fn journal_listing_without_cleanup_keeps_old_temporary_files() {
        let dir = TestDir::new();
        let path = dir
            .0
            .join("20261024T090000-journal-deadbeef.json.feedface.tmp");
        secure_new(&path).unwrap();
        let modified = SystemTime::UNIX_EPOCH + Duration::from_secs(1);
        File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(modified))
            .unwrap();

        list_journal_files(&dir.0, None).unwrap();

        assert!(path.exists());
    }

    #[test]
    fn try_lock_creates_nothing_for_a_missing_folder() {
        let dir = TestDir::new();
        let missing = dir.0.join("backups");

        assert!(try_lock_journal_in(&missing).unwrap().is_none());
        assert!(!missing.exists());
    }

    #[test]
    fn try_lock_reports_a_held_lock_without_waiting() {
        let dir = TestDir::new();
        let held = try_lock_journal_in(&dir.0).unwrap();
        assert!(held.is_some());
        assert!(try_lock_journal_in(&dir.0).unwrap().is_none());

        drop(held);
        assert!(try_lock_journal_in(&dir.0).unwrap().is_some());
    }

    #[test]
    fn config_lock_is_exclusive_until_dropped() {
        let dir = TestDir::new();
        let locks = dir.0.join("backups");
        let config = dir.0.join("config.toml");
        std::fs::write(&config, "").unwrap();

        let first = config_lock(&config, &locks).unwrap();
        let error = config_lock(&config, &locks).err().unwrap();
        assert!(
            matches!(&error, ConfigLockError::Held { path } if *path == std::fs::canonicalize(&config).unwrap()),
            "{error:?}"
        );

        drop(first);
        let _second = config_lock(&config, &locks).unwrap();
    }

    #[test]
    fn config_lock_keeps_its_file_and_reuses_one_left_by_a_crash() {
        let dir = TestDir::new();
        let config = dir.0.join("config.toml");
        std::fs::write(&config, "").unwrap();
        let lock_file = dir
            .0
            .join(config_lock_name(&std::fs::canonicalize(&config).unwrap()));
        std::fs::write(&lock_file, "").unwrap();

        drop(config_lock(&config, &dir.0).unwrap());

        assert!(lock_file.exists());
        let _again = config_lock(&config, &dir.0).unwrap();
    }

    #[test]
    fn config_lock_names_a_missing_file_by_its_canonical_folder() {
        let dir = TestDir::new();
        let config = dir.0.join("config.toml");
        let before = config_lock_target(&config).unwrap();
        std::fs::write(&config, "").unwrap();

        assert_eq!(before, config_lock_target(&config).unwrap());
    }

    #[test]
    fn config_lock_names_are_stable_and_distinct() {
        assert_eq!(fnv1a_64(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a_64(b"a"), 0xaf63_dc4c_8601_ec8c);
        assert_eq!(
            config_lock_name(Path::new("/Users/me/.config/nib/config.toml")),
            config_lock_name(Path::new("/Users/me/.config/nib/config.toml"))
        );
        assert_ne!(
            config_lock_name(Path::new("/Users/me/project/nib.toml")),
            config_lock_name(Path::new("/Users/me/project/.nib.local.toml"))
        );
        assert!(
            config_lock_name(Path::new("/Users/me/project/nib.toml"))
                .strip_prefix("config-")
                .and_then(|name| name.strip_suffix(".lock"))
                .is_some_and(|hash| hash.len() == 16)
        );
    }

    #[test]
    #[should_panic(expected = "journal lock is already held")]
    fn scripted_host_rejects_a_second_journal_lock() {
        let mut host = Scripted::new();
        let _first = host.lock_journal().unwrap();

        let _second = host.lock_journal();
    }

    #[test]
    fn removing_a_journal_warns_but_succeeds_when_the_directory_sync_fails() {
        let dir = TestDir::new();
        let path = dir.0.join("entry.json");
        secure_new(&path).unwrap();

        let warning = remove_journal_in(&dir.0, "entry", |_| {
            Err(io::Error::other("injected directory sync failure"))
        })
        .unwrap()
        .unwrap();

        assert!(!path.exists());
        assert!(warning.contains("was removed"));
        assert!(warning.contains("injected directory sync failure"));
    }
}
