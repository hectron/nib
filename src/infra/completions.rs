use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cli::CompletionKind;

const CACHE_FILE: &str = "completions.json";
const FRESH_FOR_SECONDS: i64 = 5 * 60;
const UNCHANGED_WRITE_AFTER_SECONDS: i64 = 60;
const REFRESH_WATCHDOG: Duration = Duration::from_secs(10);
const STALE_TEMPORARY_AFTER: Duration = Duration::from_secs(5 * 60);
static TEMPORARY_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone)]
pub struct CacheFile {
    path: Option<PathBuf>,
    now: i64,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct Cache {
    reminder_lists: Entry,
    writable_reminder_lists: Entry,
    calendars: Entry,
    note_folders: Entry,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct Entry {
    updated_at: i64,
    checked_at: i64,
    names: Vec<String>,
}

impl Entry {
    fn is_fresh(&self, now: i64) -> bool {
        timestamp_is_recent(self.updated_at, now)
    }

    fn was_checked_recently(&self, now: i64) -> bool {
        timestamp_is_recent(self.checked_at, now)
    }

    fn needs_refresh(&self, now: i64) -> bool {
        !self.is_fresh(now) && !self.was_checked_recently(now)
    }
}

impl CacheFile {
    pub fn new(directory: Option<&Path>, now: i64) -> Self {
        Self {
            path: directory.map(|directory| directory.join(CACHE_FILE)),
            now,
        }
    }

    pub fn update_reminder_lists(&self, names: Vec<String>, writable: Vec<String>) {
        self.update(|cache| {
            let lists_changed = replace_names(&mut cache.reminder_lists, self.now, names);
            let writable_changed =
                replace_names(&mut cache.writable_reminder_lists, self.now, writable);
            lists_changed || writable_changed
        });
    }

    pub fn is_enabled(&self) -> bool {
        self.exists()
    }

    pub fn update_calendars(&self, names: Vec<String>) {
        self.update(|cache| replace_names(&mut cache.calendars, self.now, names));
    }

    pub fn update_note_folders(&self, names: Vec<String>) {
        self.update(|cache| replace_names(&mut cache.note_folders, self.now, names));
    }

    pub fn record_note_folder(&self, name: String) {
        self.update(|cache| {
            let mut names = cache.note_folders.names.clone();
            names.push(name);
            replace_names(&mut cache.note_folders, self.now, names)
        });
    }

    pub fn values(&self, kind: CompletionKind) -> Vec<String> {
        let cache = self.read();
        if !self.exists() {
            let _ = self.write(&cache);
        }
        if matches!(
            kind,
            CompletionKind::ReminderLists
                | CompletionKind::WritableReminderLists
                | CompletionKind::Calendars
        ) && self.exists()
            && entry(&cache, kind).needs_refresh(self.now)
        {
            spawn_refresh(kind);
        }
        clean(entry(&cache, kind).names.clone())
    }

    pub fn refresh(&self, kind: CompletionKind) {
        if !self.exists() {
            return;
        }
        if !matches!(
            kind,
            CompletionKind::ReminderLists
                | CompletionKind::WritableReminderLists
                | CompletionKind::Calendars
        ) {
            return;
        }
        self.force_update(|cache| mark_checked(cache, kind, self.now));

        #[cfg(target_os = "macos")]
        match kind {
            CompletionKind::ReminderLists | CompletionKind::WritableReminderLists => {
                if let Some((names, writable)) = super::eventkit::completion_reminder_list_names() {
                    self.update(|cache| {
                        let names_changed =
                            replace_names(&mut cache.reminder_lists, self.now, names);
                        let writable_changed =
                            replace_names(&mut cache.writable_reminder_lists, self.now, writable);
                        names_changed || writable_changed
                    });
                }
            }
            CompletionKind::Calendars => {
                if let Some(names) = super::eventkit::completion_calendar_names() {
                    self.update(|cache| replace_names(&mut cache.calendars, self.now, names));
                }
            }
            CompletionKind::NoteFolders => {}
        }
    }

    fn exists(&self) -> bool {
        self.path.as_deref().is_some_and(Path::is_file)
    }

    /// Concurrent read-modify-write operations can lose one section's update. This is harmless
    /// for completion data because a later command or refresh repopulates the section.
    fn update(&self, change: impl FnOnce(&mut Cache) -> bool) {
        if !self.exists() {
            return;
        }
        let mut cache = self.read();
        if change(&mut cache) {
            let _ = self.write(&cache);
        }
    }

    fn force_update(&self, change: impl FnOnce(&mut Cache)) {
        if !self.exists() {
            return;
        }
        let mut cache = self.read();
        change(&mut cache);
        let _ = self.write(&cache);
    }

    fn read(&self) -> Cache {
        self.path
            .as_deref()
            .and_then(|path| fs::read(path).ok())
            .and_then(|contents| serde_json::from_slice(&contents).ok())
            .unwrap_or_default()
    }

    fn write(&self, cache: &Cache) -> io::Result<()> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        let Some(directory) = path.parent() else {
            return Ok(());
        };
        fs::create_dir_all(directory)?;
        remove_stale_temporary_files(directory);
        let temporary = temporary_path(path);
        let result = write_temporary(&temporary, cache).and_then(|()| fs::rename(&temporary, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

pub fn values(kind: CompletionKind, directory: Option<&Path>, now: i64, demo: bool) -> Vec<String> {
    if demo {
        return demo_values(kind);
    }
    CacheFile::new(directory, now).values(kind)
}

pub fn refresh(kind: CompletionKind, directory: Option<&Path>, now: i64, demo: bool) {
    if demo {
        return;
    }
    let _ = std::thread::spawn(|| {
        std::thread::sleep(REFRESH_WATCHDOG);
        std::process::exit(0);
    });
    #[cfg(target_os = "macos")]
    super::eventkit::start_new_session();
    CacheFile::new(directory, now).refresh(kind);
}

fn timestamp_is_recent(timestamp: i64, now: i64) -> bool {
    timestamp_is_within(timestamp, now, FRESH_FOR_SECONDS)
}

fn timestamp_is_within(timestamp: i64, now: i64, seconds: i64) -> bool {
    timestamp != 0 && timestamp <= now && now.saturating_sub(timestamp) <= seconds
}

fn entry(cache: &Cache, kind: CompletionKind) -> &Entry {
    match kind {
        CompletionKind::ReminderLists => &cache.reminder_lists,
        CompletionKind::WritableReminderLists => &cache.writable_reminder_lists,
        CompletionKind::Calendars => &cache.calendars,
        CompletionKind::NoteFolders => &cache.note_folders,
    }
}

fn mark_checked(cache: &mut Cache, kind: CompletionKind, now: i64) {
    match kind {
        CompletionKind::ReminderLists | CompletionKind::WritableReminderLists => {
            cache.reminder_lists.checked_at = now;
            cache.writable_reminder_lists.checked_at = now;
        }
        CompletionKind::Calendars => cache.calendars.checked_at = now,
        CompletionKind::NoteFolders => {}
    }
}

fn replace_names(target: &mut Entry, now: i64, names: Vec<String>) -> bool {
    let names = clean(names);
    if target.names == names {
        if timestamp_is_within(target.checked_at, now, UNCHANGED_WRITE_AFTER_SECONDS) {
            return false;
        }
        target.checked_at = now;
        return true;
    }
    target.names = names;
    target.updated_at = now;
    target.checked_at = now;
    true
}

fn clean(mut names: Vec<String>) -> Vec<String> {
    names.retain(|name| !name.is_empty() && !name.chars().any(char::is_control));
    names.sort();
    names.dedup();
    names
}

fn demo_values(kind: CompletionKind) -> Vec<String> {
    match kind {
        CompletionKind::ReminderLists | CompletionKind::WritableReminderLists => {
            vec!["Reminders".into()]
        }
        CompletionKind::Calendars | CompletionKind::NoteFolders => {
            vec!["Personal".into(), "Work".into()]
        }
    }
}

fn spawn_refresh(kind: CompletionKind) {
    let Ok(executable) = std::env::current_exe() else {
        return;
    };
    let _ = Command::new(executable)
        .args(["__complete", "--refresh", kind.as_str()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
}

fn temporary_path(path: &Path) -> PathBuf {
    let id = TEMPORARY_ID.fetch_add(1, Ordering::Relaxed);
    path.with_extension(format!("tmp-{}-{id}", std::process::id()))
}

fn remove_stale_temporary_files(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let is_stale = entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.starts_with("completions.tmp-"))
            && entry
                .metadata()
                .and_then(|metadata| metadata.modified())
                .and_then(|modified| modified.elapsed().map_err(io::Error::other))
                .is_ok_and(|age| age >= STALE_TEMPORARY_AFTER);
        if is_stale {
            let _ = fs::remove_file(path);
        }
    }
}

fn write_temporary(path: &Path, cache: &Cache) -> io::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    serde_json::to_writer(&mut file, cache).map_err(io::Error::other)?;
    file.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::SystemTime;

    use super::*;

    fn directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("nib-completions-{name}-{}", std::process::id()))
    }

    fn reset(path: &Path) {
        if path.exists() {
            fs::remove_dir_all(path).unwrap();
        }
    }

    fn create_cache(directory: &Path, now: i64) -> CacheFile {
        let cache = CacheFile::new(Some(directory), now);
        assert_eq!(
            cache.values(CompletionKind::NoteFolders),
            Vec::<String>::new()
        );
        cache
    }

    #[test]
    fn freshness_expires_and_rejects_future_timestamps() {
        let entry = Entry {
            updated_at: 1_000,
            ..Entry::default()
        };
        assert!(entry.is_fresh(1_300));
        assert!(!entry.is_fresh(1_301));
        assert!(!entry.is_fresh(999));
        assert!(!Entry::default().is_fresh(1));
    }

    #[test]
    fn missing_cache_sections_use_defaults() {
        let cache: Cache = serde_json::from_str(r#"{"note_folders":{"names":["Work"]}}"#).unwrap();
        assert_eq!(cache.note_folders.names, ["Work"]);
        assert_eq!(cache.reminder_lists.names, Vec::<String>::new());
        assert_eq!(cache.writable_reminder_lists.names, Vec::<String>::new());
    }

    #[test]
    fn normal_updates_require_opt_in_and_throttle_unchanged_freshness_writes() {
        let directory = directory("opt-in");
        reset(&directory);
        let cache = CacheFile::new(Some(&directory), 1_000);
        cache.update_calendars(vec!["Work".into()]);
        assert!(!directory.exists());

        let cache = create_cache(&directory, 1_000);
        cache.update_calendars(vec!["Work".into()]);
        let path = directory.join(CACHE_FILE);
        let before = fs::read(&path).unwrap();
        CacheFile::new(Some(&directory), 1_060).update_calendars(vec!["Work".into()]);
        assert_eq!(fs::read(&path).unwrap(), before);
        CacheFile::new(Some(&directory), 1_061).update_calendars(vec!["Work".into()]);
        let stored: Cache = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(stored.calendars.updated_at, 1_000);
        assert_eq!(stored.calendars.checked_at, 1_061);
        reset(&directory);
    }

    #[test]
    fn reminder_refresh_checks_both_caches() {
        let directory = directory("reminder-refresh-check");
        reset(&directory);
        let cache = create_cache(&directory, 1_000);
        cache.force_update(|stored| {
            stored.reminder_lists.checked_at = 100;
            stored.writable_reminder_lists.checked_at = 200;
        });
        cache.force_update(|stored| {
            mark_checked(stored, CompletionKind::WritableReminderLists, cache.now);
        });
        let stored = cache.read();
        assert_eq!(stored.reminder_lists.checked_at, 1_000);
        assert_eq!(stored.writable_reminder_lists.checked_at, 1_000);
        reset(&directory);
    }

    #[test]
    fn writes_atomically_and_preserves_other_sections() {
        let directory = directory("atomic");
        reset(&directory);
        let cache = create_cache(&directory, 1_000);
        cache.update_reminder_lists(
            vec!["Work".into(), "Reminders".into(), "Read Only".into()],
            vec!["Work".into(), "Reminders".into()],
        );
        cache.update_calendars(vec!["Personal".into()]);

        let contents = fs::read(directory.join(CACHE_FILE)).unwrap();
        let stored: Cache = serde_json::from_slice(&contents).unwrap();
        assert_eq!(
            stored.reminder_lists.names,
            ["Read Only", "Reminders", "Work"]
        );
        assert_eq!(stored.writable_reminder_lists.names, ["Reminders", "Work"]);
        assert_eq!(stored.calendars.names, ["Personal"]);
        assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);
        reset(&directory);
    }

    #[test]
    fn a_write_removes_stale_temporary_files() {
        let directory = directory("stale-temporary");
        reset(&directory);
        let cache = create_cache(&directory, 1_000);
        let temporary = directory.join("completions.tmp-stale");
        fs::write(&temporary, b"partial names").unwrap();
        let times = fs::FileTimes::new().set_modified(
            SystemTime::now()
                .checked_sub(STALE_TEMPORARY_AFTER + Duration::from_secs(1))
                .unwrap(),
        );
        fs::File::options()
            .write(true)
            .open(&temporary)
            .unwrap()
            .set_times(times)
            .unwrap();

        cache.update_calendars(vec!["Work".into()]);
        assert!(!temporary.exists());
        reset(&directory);
    }

    #[test]
    #[cfg(unix)]
    fn cache_file_is_private() {
        let directory = directory("permissions");
        reset(&directory);
        create_cache(&directory, 1_000);
        let mode = fs::metadata(directory.join(CACHE_FILE))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        reset(&directory);
    }

    #[test]
    fn demo_values_do_not_create_a_cache() {
        let directory = directory("demo");
        reset(&directory);
        assert_eq!(
            values(CompletionKind::Calendars, Some(&directory), 1_000, true),
            ["Personal", "Work"]
        );
        assert!(!directory.exists());
    }
}
