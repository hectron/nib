use std::collections::hash_map::RandomState;
use std::ffi::OsString;
use std::fs::{File, OpenOptions};
use std::hash::BuildHasher;
use std::io::{self, BufRead, IsTerminal, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};

pub trait Host {
    fn create_edit_file(&mut self, contents: &str) -> io::Result<EditFile> {
        EditFile::create(contents)
    }

    fn edit(&mut self, path: &Path) -> io::Result<Duration>;
    fn ask(&mut self, prompt: &str) -> io::Result<String>;
    fn say(&mut self, message: &str);
    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
}

pub struct System;

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
    atomic_write_with_sync(path, contents, before_rename, |directory| {
        File::open(directory)?.sync_all()
    })
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
    remove_stale_temporary_files(directory, file_name, SystemTime::now())?;

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

fn remove_stale_temporary_files(
    directory: &Path,
    file_name: &str,
    now: SystemTime,
) -> io::Result<()> {
    let prefix = format!(".{file_name}.nib-");
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if name.starts_with(&prefix)
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension == "tmp")
        {
            let metadata = entry.metadata()?;
            let old = metadata
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > Duration::from_hours(24));
            if old {
                std::fs::remove_file(entry.path())?;
            }
        }
    }
    Ok(())
}

pub(crate) struct ConfigLock {
    path: PathBuf,
    _file: File,
}

impl Drop for ConfigLock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub(crate) fn config_lock(path: &Path) -> io::Result<ConfigLock> {
    let target = resolved_target(path)?;
    let file_name = target
        .file_name()
        .map_or_else(|| OsString::from("config"), std::ffi::OsStr::to_os_string);
    let mut lock_name = file_name;
    lock_name.push(".lock");
    let lock_path = target.with_file_name(lock_name);
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o666)
        .open(&lock_path)
    {
        Ok(file) => Ok(ConfigLock {
            path: lock_path,
            _file: file,
        }),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!(
                "another nib config command is running, or remove {} if it's stale",
                lock_path.display()
            ),
        )),
        Err(error) => Err(error),
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
        let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME isn't set"))?;
        let dir = PathBuf::from(home).join("Library/Application Support/nib/backups");
        std::fs::create_dir_all(&dir)?;
        save_backup_in(&dir, name, contents)
    }
}

#[cfg(test)]
pub struct Scripted {
    edits: std::collections::VecDeque<Box<dyn FnOnce(String) -> String>>,
    answers: std::collections::VecDeque<String>,
    pub said: Vec<String>,
    pub backups: Vec<(String, String)>,
}

#[cfg(test)]
impl Scripted {
    pub fn new() -> Self {
        Self {
            edits: std::collections::VecDeque::default(),
            answers: std::collections::VecDeque::default(),
            said: Vec::new(),
            backups: Vec::new(),
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
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(SystemTime::now() - Duration::from_hours(25)),
            )
            .unwrap();

        atomic_write(&path, "new").unwrap();

        assert!(!old.exists());
        assert!(recent.exists());
        assert!(unrelated.exists());
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
}
