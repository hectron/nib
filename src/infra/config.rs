use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;

use miette::{Diagnostic, NamedSource, SourceSpan};
use serde::Deserialize;
use thiserror::Error;
use toml_edit::{Array, Decor, DocumentMut, Item, Table, Value};

use crate::domain::keymap::{Action as KeymapAction, KeyChord, Keymap, State};
use crate::domain::{Config, ConfigEntry, ConfigKey, ConfigOrigin, ConfigValue, Priority};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileConfig {
    task: Option<FileTask>,
    cal: Option<FileCal>,
    note: Option<FileNote>,
    tui: Option<FileTui>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTask {
    list: Option<String>,
    priority: Option<Priority>,
    parse_inline: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileCal {
    calendars: Option<Vec<String>>,
    show_calendar: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileNote {
    folder: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileTui {
    preset: Option<toml::Value>,
    keys: Option<BTreeMap<String, toml::Value>>,
    views: Option<BTreeMap<String, toml::Value>>,
}

#[derive(Debug, Error, Diagnostic)]
pub enum ConfigError {
    #[error("couldn't read configuration file {}: {source}", path.display())]
    #[diagnostic(code(nib::config::read))]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("invalid configuration in {}: {reason}", path.display())]
    #[diagnostic(code(nib::config::invalid_file))]
    Parse {
        path: PathBuf,
        reason: String,
        line: usize,
        column: usize,
        #[source_code]
        source_code: Arc<NamedSource<String>>,
        #[label("{reason}")]
        span: SourceSpan,
    },

    #[error("invalid keymap in {} for {key}: {reason}", path.display())]
    #[diagnostic(code(nib::config::invalid_keymap))]
    Keymap {
        path: PathBuf,
        key: String,
        reason: String,
        line: usize,
        column: usize,
        #[source_code]
        source_code: Arc<NamedSource<String>>,
        #[label("{reason}")]
        span: SourceSpan,
    },

    #[error("invalid saved view in {} for {key}: {reason}", path.display())]
    #[diagnostic(code(nib::config::invalid_view))]
    View {
        path: PathBuf,
        key: String,
        reason: String,
        line: usize,
        column: usize,
        #[source_code]
        source_code: Arc<NamedSource<String>>,
        #[label("{reason}")]
        span: SourceSpan,
    },

    #[error("invalid value for {name}: {value:?}; expected {expected}")]
    #[diagnostic(
        code(nib::config::invalid_environment),
        help("run `nib doctor` for configuration diagnostics")
    )]
    Environment {
        name: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("{name} contains text that isn't valid UTF-8")]
    #[diagnostic(
        code(nib::config::invalid_environment),
        help("run `nib doctor` for configuration diagnostics")
    )]
    EnvironmentEncoding { name: &'static str },
}

impl ConfigError {
    fn plain_text(&self) -> String {
        match self {
            Self::Parse { line, column, .. }
            | Self::Keymap { line, column, .. }
            | Self::View { line, column, .. } => {
                format!("{self} at line {line}, column {column}")
            }
            _ => self.to_string(),
        }
    }
}

#[derive(Debug, Error, Diagnostic)]
pub enum ConfigCommandError {
    #[error(transparent)]
    #[diagnostic(transparent)]
    Config(#[from] ConfigError),

    #[error("unknown configuration key {key:?}")]
    #[diagnostic(code(nib::config::unknown_key), help("{help}"))]
    UnknownKey { key: String, help: String },

    #[error("invalid value for {key}: {value:?}; expected {expected}")]
    #[diagnostic(code(nib::config::invalid_value))]
    InvalidValue {
        key: &'static str,
        value: String,
        expected: &'static str,
    },

    #[error("configuration section {section:?} isn't a table")]
    #[diagnostic(code(nib::config::invalid_file))]
    InvalidSection { section: &'static str },

    #[error(
        "the user configuration path is unavailable because neither XDG_CONFIG_HOME nor HOME is set"
    )]
    #[diagnostic(code(nib::config::path))]
    MissingUserPath,

    #[error("couldn't create configuration directory {}: {source}", path.display())]
    #[diagnostic(code(nib::config::write))]
    CreateDirectory {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("couldn't write configuration file {}: {source}", path.display())]
    #[diagnostic(code(nib::config::write))]
    Write {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("HOME isn't set, so nib can't lock the configuration")]
    #[diagnostic(code(nib::config::lock))]
    MissingHome,

    #[error("couldn't lock configuration with {}", path.display())]
    #[diagnostic(code(nib::config::lock))]
    Lock {
        path: PathBuf,
        source: std::io::Error,
    },

    #[error("another nib config command is changing {}; try again when it finishes", path.display())]
    #[diagnostic(code(nib::config::locked))]
    Locked { path: PathBuf },
}

#[derive(Debug)]
pub struct ConfigInspection {
    result: Result<(Config, Origins), ConfigError>,
    files: Vec<ConfigFileInspection>,
    start: PathBuf,
    user_config: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConfigFileInspection {
    path: PathBuf,
    error: Option<String>,
}

impl ConfigInspection {
    pub fn into_config(self) -> Result<Config, ConfigError> {
        self.result.map(|(config, _)| config)
    }

    pub(crate) fn legacy_config_lock_files(&self) -> Vec<PathBuf> {
        let mut candidates = Vec::new();
        if let Some(user) = &self.user_config {
            candidates.push(user.clone());
        }
        if let Ok(project) = project_config_path(&self.start) {
            candidates.push(project);
        }
        candidates.push(self.start.join(".nib.local.toml"));

        let mut locks: Vec<PathBuf> = Vec::new();
        for candidate in candidates {
            let Ok(lock) = crate::infra::host::legacy_config_lock_path(&candidate) else {
                continue;
            };
            if !locks.contains(&lock) {
                locks.push(lock);
            }
        }
        locks.retain(|lock| fs::symlink_metadata(lock).is_ok());
        locks
    }

    pub(crate) fn error(&self) -> Option<&ConfigError> {
        self.result.as_ref().err()
    }

    pub(crate) fn keymap(&self) -> Option<&Keymap> {
        self.result
            .as_ref()
            .ok()
            .map(|(config, _)| &config.tui.keymap)
    }

    pub(crate) fn files(&self) -> &[ConfigFileInspection] {
        &self.files
    }

    pub(crate) fn into_entries(self) -> Result<Vec<ConfigEntry>, ConfigError> {
        self.result
            .map(|(config, origins)| origins.entries(&config))
    }

    pub(crate) fn origin(&self, key: ConfigKey) -> Option<&ConfigOrigin> {
        self.result
            .as_ref()
            .ok()
            .map(|(_, origins)| origins.get(key))
    }
}

#[derive(Debug, Clone)]
struct Origins {
    task_list: ConfigOrigin,
    task_priority: ConfigOrigin,
    task_parse_inline: ConfigOrigin,
    cal_calendars: ConfigOrigin,
    cal_show_calendar: ConfigOrigin,
    note_folder: ConfigOrigin,
    tui_preset: ConfigOrigin,
    keymap_layers: Vec<CompiledBindings>,
}

impl Default for Origins {
    fn default() -> Self {
        Self {
            task_list: ConfigOrigin::Default,
            task_priority: ConfigOrigin::Default,
            task_parse_inline: ConfigOrigin::Default,
            cal_calendars: ConfigOrigin::Default,
            cal_show_calendar: ConfigOrigin::Default,
            note_folder: ConfigOrigin::Default,
            tui_preset: ConfigOrigin::Default,
            keymap_layers: Vec::new(),
        }
    }
}

impl Origins {
    fn get(&self, key: ConfigKey) -> &ConfigOrigin {
        match key {
            ConfigKey::TaskList => &self.task_list,
            ConfigKey::TaskPriority => &self.task_priority,
            ConfigKey::TaskParseInline => &self.task_parse_inline,
            ConfigKey::CalCalendars => &self.cal_calendars,
            ConfigKey::CalShowCalendar => &self.cal_show_calendar,
            ConfigKey::NoteFolder => &self.note_folder,
            ConfigKey::TuiPreset => &self.tui_preset,
        }
    }

    fn entries(self, config: &Config) -> Vec<ConfigEntry> {
        vec![
            ConfigEntry {
                key: ConfigKey::TaskList,
                value: config
                    .task
                    .list
                    .clone()
                    .map_or(ConfigValue::Unset, ConfigValue::String),
                origin: self.task_list,
            },
            ConfigEntry {
                key: ConfigKey::TaskPriority,
                value: config
                    .task
                    .priority
                    .map_or(ConfigValue::Unset, ConfigValue::Priority),
                origin: self.task_priority,
            },
            ConfigEntry {
                key: ConfigKey::TaskParseInline,
                value: ConfigValue::Bool(config.task.parse_inline),
                origin: self.task_parse_inline,
            },
            ConfigEntry {
                key: ConfigKey::CalCalendars,
                value: ConfigValue::Strings(config.cal.calendars.clone()),
                origin: self.cal_calendars,
            },
            ConfigEntry {
                key: ConfigKey::CalShowCalendar,
                value: ConfigValue::Bool(config.cal.show_calendar),
                origin: self.cal_show_calendar,
            },
            ConfigEntry {
                key: ConfigKey::NoteFolder,
                value: config
                    .note
                    .folder
                    .clone()
                    .map_or(ConfigValue::Unset, ConfigValue::String),
                origin: self.note_folder,
            },
            ConfigEntry {
                key: ConfigKey::TuiPreset,
                value: ConfigValue::String(config.tui.preset.clone()),
                origin: self.tui_preset,
            },
        ]
    }
}

impl ConfigFileInspection {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

pub fn load(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<Config, ConfigError> {
    inspect(start, user_config, env).into_config()
}

pub fn inspect(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
) -> ConfigInspection {
    let mut files = Vec::new();
    ConfigInspection {
        result: load_inspected(start, user_config, env, &mut files),
        files,
        start: start.to_owned(),
        user_config: user_config.map(Path::to_owned),
    }
}

fn load_inspected(
    start: &Path,
    user_config: Option<&Path>,
    env: impl Fn(&str) -> Option<OsString>,
    files: &mut Vec<ConfigFileInspection>,
) -> Result<(Config, Origins), ConfigError> {
    let mut config = Config::default();
    let mut origins = Origins::default();
    if let Some(path) = user_config {
        apply_if_present(&mut config, &mut origins, path, files)?;
    }

    let directories: Vec<&Path> = start.ancestors().collect();
    for directory in directories.into_iter().rev() {
        let mut project = project_config_path(directory)?;
        if !is_distinct(&project, user_config) {
            let hidden = directory.join(".nib.toml");
            if is_distinct(&hidden, user_config) && exists(&hidden)? {
                project = hidden;
            }
        }
        if is_distinct(&project, user_config) && exists(&project)? {
            apply_file_inspected(&mut config, &mut origins, &project, files)?;
        }

        let local = directory.join(".nib.local.toml");
        if is_distinct(&local, user_config) {
            apply_if_present(&mut config, &mut origins, &local, files)?;
        }
    }

    apply_environment(&mut config, &mut origins, env)?;
    config.tui.keymap = build_keymap(&config.tui.preset, &origins.keymap_layers);
    Ok((config, origins))
}

pub(crate) fn project_config_path(directory: &Path) -> Result<PathBuf, ConfigError> {
    let visible = directory.join("nib.toml");
    if exists(&visible)? {
        return Ok(visible);
    }
    let hidden = directory.join(".nib.toml");
    if exists(&hidden)? {
        return Ok(hidden);
    }
    Ok(visible)
}

fn is_distinct(path: &Path, user_config: Option<&Path>) -> bool {
    user_config.is_none_or(|user| path != user)
}

fn apply_if_present(
    config: &mut Config,
    origins: &mut Origins,
    path: &Path,
    files: &mut Vec<ConfigFileInspection>,
) -> Result<(), ConfigError> {
    if exists(path)? {
        apply_file_inspected(config, origins, path, files)?;
    }
    Ok(())
}

fn apply_file_inspected(
    config: &mut Config,
    origins: &mut Origins,
    path: &Path,
    files: &mut Vec<ConfigFileInspection>,
) -> Result<(), ConfigError> {
    match apply_file(config, origins, path) {
        Ok(()) => {
            files.push(ConfigFileInspection {
                path: path.to_owned(),
                error: None,
            });
            Ok(())
        }
        Err(error) => {
            files.push(ConfigFileInspection {
                path: path.to_owned(),
                error: Some(error.plain_text()),
            });
            Err(error)
        }
    }
}

fn exists(path: &Path) -> Result<bool, ConfigError> {
    match fs::metadata(path) {
        Ok(_) => Ok(true),
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(ConfigError::Read {
            path: path.to_owned(),
            source,
        }),
    }
}

fn apply_file(config: &mut Config, origins: &mut Origins, path: &Path) -> Result<(), ConfigError> {
    let source = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    let parsed = parse_file(path, &source)?;
    apply(config, origins, parsed, ConfigOrigin::File(path.to_owned()))
        .map_err(|issue| keymap_error(path, &source, issue))?;
    Ok(())
}

fn parse_file(path: &Path, source: &str) -> Result<FileConfig, ConfigError> {
    toml::from_str(source).map_err(|error: toml::de::Error| {
        let span = error.span().unwrap_or(0..0);
        let (line, column) = source_location(source, span.start);
        ConfigError::Parse {
            path: path.to_owned(),
            reason: error.message().to_owned(),
            line,
            column,
            source_code: Arc::new(
                NamedSource::new(path.display().to_string(), source.to_owned())
                    .with_language("toml"),
            ),
            span: span.into(),
        }
    })
}

fn apply(
    config: &mut Config,
    origins: &mut Origins,
    other: FileConfig,
    origin: ConfigOrigin,
) -> Result<(), KeymapIssue> {
    let tui = other.tui.map(compile_tui).transpose()?;
    if let Some(task) = other.task {
        if task.list.is_some() {
            config.task.list = task.list;
            origins.task_list = origin.clone();
        }
        if task.priority.is_some() {
            config.task.priority = task.priority;
            origins.task_priority = origin.clone();
        }
        if let Some(parse_inline) = task.parse_inline {
            config.task.parse_inline = parse_inline;
            origins.task_parse_inline = origin.clone();
        }
    }
    if let Some(cal) = other.cal {
        if let Some(calendars) = cal.calendars {
            config.cal.calendars = calendars;
            origins.cal_calendars = origin.clone();
        }
        if let Some(show_calendar) = cal.show_calendar {
            config.cal.show_calendar = show_calendar;
            origins.cal_show_calendar = origin.clone();
        }
    }
    if let Some(note) = other.note
        && note.folder.is_some()
    {
        config.note.folder = note.folder;
        origins.note_folder = origin.clone();
    }
    if let Some(tui) = tui {
        if let Some(preset) = tui.preset {
            config.tui.preset = preset;
            origins.tui_preset = origin;
        }
        origins.keymap_layers.push(tui.bindings);
        config.tui.views.extend(tui.views);
    }
    Ok(())
}

fn build_keymap(preset: &str, layers: &[CompiledBindings]) -> Keymap {
    let mut keymap =
        Keymap::from_preset(preset).expect("presets are validated before they are stored");
    for (state, bindings) in layers.iter().flatten() {
        for (chord, actions) in bindings {
            keymap.bind(*state, chord.clone(), actions.clone());
        }
    }
    keymap
}

struct CompiledTui {
    preset: Option<String>,
    bindings: CompiledBindings,
    views: BTreeMap<String, String>,
}

type CompiledBindings = Vec<(State, StateBindings)>;
type StateBindings = Vec<(KeyChord, Vec<KeymapAction>)>;

struct KeymapIssue {
    key: String,
    reason: String,
    needle: String,
    span: Box<KeymapSpan>,
}

enum KeymapSpan {
    Preset,
    State(String),
    Chord { state: String, chord: String },
    Value { state: String, chord: String },
    View(String),
}

fn compile_tui(tui: FileTui) -> Result<CompiledTui, KeymapIssue> {
    let preset = tui.preset.map(parse_preset).transpose()?;
    let mut compiled = Vec::new();
    for (state_name, value) in tui.keys.unwrap_or_default() {
        let state = state_name.parse::<State>().map_err(|error| KeymapIssue {
            key: format!("tui.keys.{state_name}"),
            reason: error.to_string(),
            needle: state_name.clone(),
            span: KeymapSpan::State(state_name.clone()).into(),
        })?;
        let toml::Value::Table(bindings) = value else {
            return Err(KeymapIssue {
                key: format!("tui.keys.{state_name}"),
                reason: "a keymap state must be a table of key chords and actions".into(),
                needle: state_name.clone(),
                span: KeymapSpan::State(state_name).into(),
            });
        };
        let mut canonical = BTreeMap::<KeyChord, String>::new();
        let mut state_bindings = Vec::new();
        for (raw_chord, value) in bindings {
            let key = format!("tui.keys.{state_name}.{raw_chord:?}");
            let chord = raw_chord.parse::<KeyChord>().map_err(|error| KeymapIssue {
                key: key.clone(),
                reason: error.to_string(),
                needle: raw_chord.clone(),
                span: KeymapSpan::Chord {
                    state: state_name.clone(),
                    chord: raw_chord.clone(),
                }
                .into(),
            })?;
            if let Some(previous) = canonical.insert(chord.clone(), raw_chord.clone()) {
                return Err(KeymapIssue {
                    key,
                    reason: format!(
                        "key chord {raw_chord:?} duplicates {previous:?} after canonicalization to {chord:?}",
                        chord = chord.to_string()
                    ),
                    needle: raw_chord.clone(),
                    span: KeymapSpan::Chord {
                        state: state_name.clone(),
                        chord: raw_chord,
                    }
                    .into(),
                });
            }
            let actions = parse_actions(&key, &state_name, &raw_chord, value)?;
            state_bindings.push((chord, actions));
        }
        compiled.push((state, state_bindings));
    }
    let views = tui
        .views
        .unwrap_or_default()
        .into_iter()
        .map(|(name, value)| parse_view(name, value))
        .collect::<Result<_, _>>()?;
    Ok(CompiledTui {
        preset,
        bindings: compiled,
        views,
    })
}

fn parse_view(name: String, value: toml::Value) -> Result<(String, String), KeymapIssue> {
    let issue = |reason: String, needle: &str| KeymapIssue {
        key: format!("tui.views.{name:?}"),
        reason,
        needle: needle.to_owned(),
        span: KeymapSpan::View(name.clone()).into(),
    };
    if name.trim().is_empty() || name.trim() != name || name.chars().any(char::is_control) {
        return Err(issue(
            "a view name must be non-empty, without control characters or surrounding spaces"
                .into(),
            &name,
        ));
    }
    let toml::Value::String(expression) = value else {
        return Err(issue(
            "a saved view must be a filter expression string".into(),
            &name,
        ));
    };
    crate::app::filter::Filter::parse(&expression)
        .map_err(|error| issue(error.to_string(), &expression))?;
    Ok((name, expression))
}

fn parse_preset(value: toml::Value) -> Result<String, KeymapIssue> {
    let toml::Value::String(preset) = value else {
        return Err(KeymapIssue {
            key: "tui.preset".into(),
            reason: "a keymap preset must be a string".into(),
            needle: "preset".into(),
            span: KeymapSpan::Preset.into(),
        });
    };
    Keymap::from_preset(&preset).map_err(|error| KeymapIssue {
        key: "tui.preset".into(),
        reason: error.to_string(),
        needle: preset.clone(),
        span: KeymapSpan::Preset.into(),
    })?;
    Ok(preset)
}

fn parse_actions(
    key: &str,
    state: &str,
    raw_chord: &str,
    value: toml::Value,
) -> Result<Vec<KeymapAction>, KeymapIssue> {
    let raw_actions = match value {
        toml::Value::String(action) => vec![action],
        toml::Value::Array(actions) => actions
            .into_iter()
            .map(|action| match action {
                toml::Value::String(action) => Ok(action),
                _ => Err(KeymapIssue {
                    key: key.into(),
                    reason: "an action sequence must contain only strings".into(),
                    needle: raw_chord.into(),
                    span: KeymapSpan::Value {
                        state: state.into(),
                        chord: raw_chord.into(),
                    }
                    .into(),
                }),
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => {
            return Err(KeymapIssue {
                key: key.into(),
                reason: "a binding must be an action string or an array of action strings".into(),
                needle: raw_chord.into(),
                span: KeymapSpan::Value {
                    state: state.into(),
                    chord: raw_chord.into(),
                }
                .into(),
            });
        }
    };
    if raw_actions.is_empty() {
        return Err(KeymapIssue {
            key: key.into(),
            reason: "an action sequence cannot be empty; use `none` to remove a binding".into(),
            needle: raw_chord.into(),
            span: KeymapSpan::Value {
                state: state.into(),
                chord: raw_chord.into(),
            }
            .into(),
        });
    }
    let mut actions = Vec::with_capacity(raw_actions.len());
    for raw_action in raw_actions {
        actions.push(raw_action.parse::<KeymapAction>().map_err(|error| {
            KeymapIssue {
                key: key.into(),
                reason: error.to_string(),
                needle: raw_action,
                span: KeymapSpan::Value {
                    state: state.into(),
                    chord: raw_chord.into(),
                }
                .into(),
            }
        })?);
    }
    if actions.len() > 1 && actions.contains(&KeymapAction::None) {
        return Err(KeymapIssue {
            key: key.into(),
            reason: "action `none` must be the only action in a binding".into(),
            needle: "none".into(),
            span: KeymapSpan::Value {
                state: state.into(),
                chord: raw_chord.into(),
            }
            .into(),
        });
    }
    Ok(actions)
}

fn keymap_error(path: &Path, source: &str, issue: KeymapIssue) -> ConfigError {
    let fallback = source
        .find(&issue.needle)
        .map_or(0..1, |offset| offset..offset + issue.needle.len().max(1));
    let span = keymap_span(source, issue.span.as_ref()).unwrap_or(fallback);
    let offset = span.start;
    let (line, column) = source_location(source, offset);
    let source_code = Arc::new(
        NamedSource::new(path.display().to_string(), source.to_owned()).with_language("toml"),
    );
    if matches!(issue.span.as_ref(), KeymapSpan::View(_)) {
        return ConfigError::View {
            path: path.to_owned(),
            key: issue.key,
            reason: issue.reason,
            line,
            column,
            source_code,
            span: span.into(),
        };
    }
    ConfigError::Keymap {
        path: path.to_owned(),
        key: issue.key,
        reason: issue.reason,
        line,
        column,
        source_code,
        span: span.into(),
    }
}

fn keymap_span(source: &str, target: &KeymapSpan) -> Option<std::ops::Range<usize>> {
    let document = DocumentMut::from_str(source).ok()?;
    let tui = document.get("tui")?.as_table_like()?;
    match target {
        KeymapSpan::Preset => tui.get("preset")?.span(),
        KeymapSpan::State(state) => tui.get("keys")?.as_table_like()?.key(state)?.span(),
        KeymapSpan::Chord { state, chord } => tui
            .get("keys")?
            .as_table_like()?
            .get(state)?
            .as_table_like()?
            .key(chord)?
            .span(),
        KeymapSpan::Value { state, chord } => tui
            .get("keys")?
            .as_table_like()?
            .get(state)?
            .as_table_like()?
            .get(chord)?
            .span(),
        KeymapSpan::View(name) => tui.get("views")?.as_table_like()?.get(name)?.span(),
    }
}

fn apply_environment(
    config: &mut Config,
    origins: &mut Origins,
    env: impl Fn(&str) -> Option<OsString>,
) -> Result<(), ConfigError> {
    if let Some(value) = environment(&env, "NIB_TASK_LIST")? {
        config.task.list = Some(value);
        origins.task_list = ConfigOrigin::Environment("NIB_TASK_LIST");
    }
    if let Some(value) = environment(&env, "NIB_TASK_PRIORITY")? {
        config.task.priority = Some(parse_priority("NIB_TASK_PRIORITY", &value)?);
        origins.task_priority = ConfigOrigin::Environment("NIB_TASK_PRIORITY");
    }
    if let Some(value) = environment(&env, "NIB_TASK_PARSE_INLINE")? {
        config.task.parse_inline = parse_bool("NIB_TASK_PARSE_INLINE", &value)?;
        origins.task_parse_inline = ConfigOrigin::Environment("NIB_TASK_PARSE_INLINE");
    }
    if let Some(value) = environment(&env, "NIB_CAL_CALENDARS")? {
        config.cal.calendars = comma_separated_names(&value);
        origins.cal_calendars = ConfigOrigin::Environment("NIB_CAL_CALENDARS");
    }
    if let Some(value) = environment(&env, "NIB_CAL_SHOW_CALENDAR")? {
        config.cal.show_calendar = parse_bool("NIB_CAL_SHOW_CALENDAR", &value)?;
        origins.cal_show_calendar = ConfigOrigin::Environment("NIB_CAL_SHOW_CALENDAR");
    }
    if let Some(value) = environment(&env, "NIB_NOTE_FOLDER")? {
        config.note.folder = Some(value);
        origins.note_folder = ConfigOrigin::Environment("NIB_NOTE_FOLDER");
    }
    if let Some(value) = environment(&env, "NIB_TUI_PRESET")? {
        Keymap::from_preset(&value).map_err(|_| ConfigError::Environment {
            name: "NIB_TUI_PRESET",
            value: value.clone(),
            expected: "default",
        })?;
        config.tui.preset = value;
        origins.tui_preset = ConfigOrigin::Environment("NIB_TUI_PRESET");
    }
    Ok(())
}

fn environment(
    env: &impl Fn(&str) -> Option<OsString>,
    name: &'static str,
) -> Result<Option<String>, ConfigError> {
    env(name)
        .map(|value| {
            value
                .into_string()
                .map_err(|_| ConfigError::EnvironmentEncoding { name })
        })
        .transpose()
}

fn parse_priority(name: &'static str, value: &str) -> Result<Priority, ConfigError> {
    priority(value).ok_or_else(|| ConfigError::Environment {
        name,
        value: value.into(),
        expected: "low, medium, or high",
    })
}

fn parse_bool(name: &'static str, value: &str) -> Result<bool, ConfigError> {
    boolean(value).ok_or_else(|| ConfigError::Environment {
        name,
        value: value.into(),
        expected: "true or false",
    })
}

fn priority(value: &str) -> Option<Priority> {
    match value {
        "low" => Some(Priority::Low),
        "medium" => Some(Priority::Medium),
        "high" => Some(Priority::High),
        _ => None,
    }
}

fn boolean(value: &str) -> Option<bool> {
    match value {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

fn comma_separated_names(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect()
}

pub(crate) fn key(value: &str) -> Result<ConfigKey, ConfigCommandError> {
    ConfigKey::parse(value).ok_or_else(|| ConfigCommandError::UnknownKey {
        key: value.to_owned(),
        help: valid_keys_help(),
    })
}

fn valid_keys_help() -> String {
    let keys = ConfigKey::ALL.map(ConfigKey::as_str);
    format!("valid keys are {}", keys.join(", "))
}

pub(crate) fn set(
    path: &Path,
    key: ConfigKey,
    input: &str,
    lock_dir: &Path,
) -> Result<ConfigEdit<ConfigValue>, ConfigCommandError> {
    let (config_value, mut edit_value) = edit_value(key, input)?;
    create_config_directory(path)?;
    let _lock = config_lock(path, lock_dir)?;
    let mut editable = editable_document(path)?.unwrap_or_else(|| EditableDocument {
        document: DocumentMut::new(),
        crlf: false,
    });
    let (section, name) = key.parts();
    let table = editable
        .document
        .entry(section)
        .or_insert(Item::Table(Table::new()))
        .as_table_like_mut()
        .ok_or(ConfigCommandError::InvalidSection { section })?;
    if let Some(existing) = table.get_mut(name).and_then(Item::as_value_mut) {
        preserve_value_format(existing, &mut edit_value);
        *existing = edit_value;
    } else {
        table.insert(name, Item::Value(edit_value));
    }
    let warning = write_document(path, &editable)?;
    Ok(ConfigEdit {
        value: config_value,
        warning,
    })
}

pub(crate) fn unset(
    path: &Path,
    key: ConfigKey,
    lock_dir: &Path,
) -> Result<ConfigEdit<bool>, ConfigCommandError> {
    match fs::symlink_metadata(path) {
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(ConfigEdit {
                value: false,
                warning: None,
            });
        }
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }
            .into());
        }
    }
    let _lock = config_lock(path, lock_dir)?;
    let Some(mut editable) = editable_document(path)? else {
        return Ok(ConfigEdit {
            value: false,
            warning: None,
        });
    };
    let (section, name) = key.parts();
    let next = next_root_key(&editable.document, section);
    let (empty, key_comments) = {
        let Some(table) = editable
            .document
            .get_mut(section)
            .and_then(Item::as_table_like_mut)
        else {
            return Ok(ConfigEdit {
                value: false,
                warning: None,
            });
        };
        let key_comments = table
            .key(name)
            .map(|key| leading_comments(key.leaf_decor()))
            .unwrap_or_default();
        if table.remove(name).is_none() {
            return Ok(ConfigEdit {
                value: false,
                warning: None,
            });
        }
        (table.is_empty(), key_comments)
    };
    if empty {
        let section_comments = if editable
            .document
            .get(section)
            .is_some_and(item_has_comments)
        {
            String::new()
        } else {
            let comments = editable
                .document
                .key(section)
                .map(|key| leading_comments(key.leaf_decor()))
                .unwrap_or_default();
            editable.document.remove(section);
            comments
        };
        let comments = format!("{section_comments}{key_comments}");
        place_comments(&mut editable.document, next.as_deref(), &comments);
    }
    let warning = write_document(path, &editable)?;
    Ok(ConfigEdit {
        value: true,
        warning,
    })
}

#[derive(Debug)]
pub(crate) struct ConfigEdit<T> {
    pub(crate) value: T,
    pub(crate) warning: Option<String>,
}

struct EditableDocument {
    document: DocumentMut,
    crlf: bool,
}

fn editable_document(path: &Path) -> Result<Option<EditableDocument>, ConfigCommandError> {
    let source = match fs::read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_owned(),
                source,
            }
            .into());
        }
    };
    parse_file(path, &source)?;
    let crlf = source.contains("\r\n");
    DocumentMut::from_str(&source)
        .map(|document| Some(EditableDocument { document, crlf }))
        .map_err(|error| {
            let span = error.span().unwrap_or(0..0);
            let (line, column) = source_location(&source, span.start);
            ConfigError::Parse {
                path: path.to_owned(),
                reason: error.message().to_owned(),
                line,
                column,
                source_code: Arc::new(
                    NamedSource::new(path.display().to_string(), source).with_language("toml"),
                ),
                span: span.into(),
            }
            .into()
        })
}

fn source_location(source: &str, offset: usize) -> (usize, usize) {
    let prefix = &source[..offset];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count() + 1;
    let column = prefix
        .rsplit_once('\n')
        .map_or(prefix, |(_, tail)| tail)
        .chars()
        .count()
        + 1;
    (line, column)
}

fn create_config_directory(path: &Path) -> Result<(), ConfigCommandError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ConfigCommandError::CreateDirectory {
            path: parent.to_owned(),
            source,
        })?;
    }
    Ok(())
}

fn config_lock(
    path: &Path,
    lock_dir: &Path,
) -> Result<crate::infra::host::ConfigLock, ConfigCommandError> {
    crate::infra::host::config_lock(path, lock_dir).map_err(|error| match error {
        crate::infra::host::ConfigLockError::Held { path } => ConfigCommandError::Locked { path },
        crate::infra::host::ConfigLockError::Target(source) => ConfigCommandError::Write {
            path: path.to_owned(),
            source,
        },
        crate::infra::host::ConfigLockError::Lock { path, source } => {
            ConfigCommandError::Lock { path, source }
        }
    })
}

fn next_root_key(document: &DocumentMut, section: &str) -> Option<String> {
    document
        .iter()
        .skip_while(|(key, _)| *key != section)
        .nth(1)
        .map(|(key, _)| key.to_owned())
}

fn leading_comments(decor: &Decor) -> String {
    decor
        .prefix()
        .and_then(toml_edit::RawString::as_str)
        .filter(|text| text.contains('#'))
        .unwrap_or_default()
        .to_owned()
}

fn place_comments(document: &mut DocumentMut, next: Option<&str>, comments: &str) {
    if comments.is_empty() {
        return;
    }
    if let Some(next) = next {
        match document.get_mut(next) {
            Some(Item::Table(table)) if !table.is_dotted() => {
                prepend_prefix(table.decor_mut(), comments);
                return;
            }
            Some(Item::Table(table)) => {
                let first = table.iter().next().map(|(key, _)| key.to_owned());
                if let Some(first) = first
                    && let Some(mut key) = table.key_mut(&first)
                {
                    prepend_prefix(key.leaf_decor_mut(), comments);
                    return;
                }
            }
            _ => {}
        }
        if let Some(mut key) = document.key_mut(next) {
            prepend_prefix(key.leaf_decor_mut(), comments);
            return;
        }
    }
    let trailing = document.trailing().as_str().unwrap_or_default().to_owned();
    let separator = if trailing.is_empty() || trailing.ends_with('\n') {
        ""
    } else {
        "\n"
    };
    document.set_trailing(format!("{trailing}{separator}{comments}"));
}

fn prepend_prefix(decor: &mut Decor, comments: &str) {
    let existing = decor
        .prefix()
        .and_then(toml_edit::RawString::as_str)
        .unwrap_or_default()
        .to_owned();
    decor.set_prefix(format!("{comments}{existing}"));
}

fn write_document(
    path: &Path,
    editable: &EditableDocument,
) -> Result<Option<String>, ConfigCommandError> {
    let mut source = editable.document.to_string();
    if editable.crlf {
        source = source.replace("\r\n", "\n").replace('\n', "\r\n");
    }
    parse_file(path, &source)?;
    let outcome = crate::infra::host::atomic_write(path, &source).map_err(|source| {
        ConfigCommandError::Write {
            path: path.to_owned(),
            source,
        }
    })?;
    Ok(outcome.directory_sync_error().map(|(directory, error)| {
        format!(
            "configuration was updated, but couldn't sync directory {}: {error}",
            directory.display()
        )
    }))
}

fn item_has_comments(item: &Item) -> bool {
    match item {
        Item::Table(table) => decor_has_comments(table.decor()),
        Item::Value(value) => decor_has_comments(value.decor()),
        Item::ArrayOfTables(_) | Item::None => false,
    }
}

fn decor_has_comments(decor: &Decor) -> bool {
    decor
        .prefix()
        .and_then(toml_edit::RawString::as_str)
        .is_some_and(|text| text.contains('#'))
        || decor
            .suffix()
            .and_then(toml_edit::RawString::as_str)
            .is_some_and(|text| text.contains('#'))
}

fn preserve_value_format(existing: &Value, replacement: &mut Value) {
    let literal = match (existing, &*replacement) {
        (Value::String(existing), Value::String(replacement)) => existing
            .display_repr()
            .starts_with('\'')
            .then(|| format!("'{}'", replacement.value())),
        _ => None,
    };
    if let Some(literal) = literal
        && let Ok(value) = Value::from_str(&literal)
    {
        *replacement = value;
    }
    replacement.decor_mut().clone_from(existing.decor());
}

fn edit_value(key: ConfigKey, input: &str) -> Result<(ConfigValue, Value), ConfigCommandError> {
    match key {
        ConfigKey::TaskList | ConfigKey::NoteFolder => Ok((
            ConfigValue::String(input.into()),
            Value::from(input.to_owned()),
        )),
        ConfigKey::TuiPreset => {
            Keymap::from_preset(input).map_err(|_| invalid_value(key, input, "default"))?;
            Ok((
                ConfigValue::String(input.into()),
                Value::from(input.to_owned()),
            ))
        }
        ConfigKey::TaskPriority => {
            let priority =
                priority(input).ok_or_else(|| invalid_value(key, input, "low, medium, or high"))?;
            Ok((ConfigValue::Priority(priority), Value::from(input)))
        }
        ConfigKey::TaskParseInline | ConfigKey::CalShowCalendar => {
            let parsed =
                boolean(input).ok_or_else(|| invalid_value(key, input, "true or false"))?;
            Ok((ConfigValue::Bool(parsed), Value::from(parsed)))
        }
        ConfigKey::CalCalendars => {
            let trimmed = input.trim();
            let parsed = if trimmed.starts_with('[') {
                let Value::Array(array) = Value::from_str(trimmed).map_err(|_| {
                    invalid_value(
                        key,
                        input,
                        "comma-separated names such as Work, Personal or a TOML array such as '[\"Work\", \"Personal\"]'",
                    )
                })?
                else {
                    return Err(invalid_value(
                        key,
                        input,
                        "comma-separated names such as Work, Personal or a TOML array such as '[\"Work\", \"Personal\"]'",
                    ));
                };
                array
                    .iter()
                    .map(|value| {
                        value.as_str().map(str::to_owned).ok_or_else(|| {
                            invalid_value(
                                key,
                                input,
                                "comma-separated names such as Work, Personal or a TOML array such as '[\"Work\", \"Personal\"]'",
                            )
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                comma_separated_names(trimmed)
            };
            let mut array = Array::new();
            array.extend(parsed.iter().cloned());
            Ok((ConfigValue::Strings(parsed), Value::Array(array)))
        }
    }
}

fn invalid_value(key: ConfigKey, value: &str, expected: &'static str) -> ConfigCommandError {
    ConfigCommandError::InvalidValue {
        key: key.as_str(),
        value: value.into(),
        expected,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashMap};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use clap::Parser;
    use jiff::civil::date;
    use jiff::tz::{TimeZone, offset};
    use miette::{GraphicalReportHandler, GraphicalTheme};

    use super::*;
    use crate::cli::{Action, Cli};
    use crate::domain::keymap::TEST_PRESET;
    use crate::domain::{EventQuery, NewReminder};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "nib-config-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn join(&self, path: impl AsRef<Path>) -> PathBuf {
            self.0.join(path)
        }

        fn locks(&self) -> PathBuf {
            self.0.join("locks")
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            fs::remove_dir_all(&self.0).unwrap();
        }
    }

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    fn no_env(_: &str) -> Option<OsString> {
        None
    }

    fn render(error: &dyn miette::Diagnostic) -> String {
        let mut rendered = String::new();
        GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
            .render_report(&mut rendered, error)
            .unwrap();
        rendered
    }

    #[test]
    fn no_configuration_keeps_built_in_defaults() {
        let temp = TempDir::new();
        assert_eq!(load(&temp.0, None, no_env).unwrap(), Config::default());
    }

    #[test]
    fn keymap_overrides_layer_by_state_and_canonical_chord() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        let user = temp.join("user.toml");
        write(
            &user,
            "[tui]\npreset = \"default\"\n[tui.keys.browse]\nx = \"none\"\n\"Alt-X\" = [\"done\", \"down\"]\n[tui.keys.filter]\n\"ctrl-o\" = \"act-menu\"\n",
        );
        write(
            &project.join("nib.toml"),
            "[tui.keys.browse]\n\"alt-x\" = \"edit\"\n[tui.keys.filter]\n\"Ctrl-O\" = \"none\"\n",
        );

        let config = load(&project, Some(&user), no_env).unwrap();
        let x = "x".parse::<KeyChord>().unwrap();
        let alt_x = "alt-x".parse::<KeyChord>().unwrap();
        let ctrl_o = "ctrl-o".parse::<KeyChord>().unwrap();
        let j = "j".parse::<KeyChord>().unwrap();
        assert_eq!(config.tui.preset, "default");
        assert_eq!(config.tui.keymap.actions(State::Browse, &x), None);
        assert_eq!(
            config.tui.keymap.actions(State::Browse, &alt_x),
            Some(&[KeymapAction::Edit][..])
        );
        assert_eq!(config.tui.keymap.actions(State::Filter, &ctrl_o), None);
        assert_eq!(
            config.tui.keymap.actions(State::Browse, &j),
            Some(&[KeymapAction::Down][..])
        );
    }

    #[test]
    fn keymap_is_built_from_the_final_preset_before_layered_overrides() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        let user = temp.join("user.toml");
        write(&user, "[tui.keys.browse]\ny = \"edit\"\n");
        write(
            &project.join("nib.toml"),
            &format!("[tui]\npreset = \"{TEST_PRESET}\"\n"),
        );
        let x = "x".parse::<KeyChord>().unwrap();
        let y = "y".parse::<KeyChord>().unwrap();
        let z = "z".parse::<KeyChord>().unwrap();
        let assert_alternate = |config: &Config| {
            assert_eq!(config.tui.preset, TEST_PRESET);
            assert_eq!(config.tui.keymap.actions(State::Browse, &x), None);
            assert_eq!(
                config.tui.keymap.actions(State::Browse, &z),
                Some(&[KeymapAction::Done][..])
            );
            assert_eq!(
                config.tui.keymap.actions(State::Browse, &y),
                Some(&[KeymapAction::Edit][..])
            );
        };

        assert_alternate(&load(&project, Some(&user), no_env).unwrap());

        fs::remove_file(project.join("nib.toml")).unwrap();
        let from_environment = load(&project, Some(&user), |name| {
            (name == "NIB_TUI_PRESET").then(|| OsString::from(TEST_PRESET))
        })
        .unwrap();
        assert_alternate(&from_environment);
    }

    #[test]
    fn keymap_errors_name_the_file_and_key_with_suggestions() {
        let cases = [
            (
                "[tui.keys.brows]\nx = \"done\"\n",
                "tui.keys.brows",
                "did you mean \"browse\"",
            ),
            (
                "[tui.keys.browse]\n\"ctrl-\" = \"done\"\n",
                "tui.keys.browse.\"ctrl-\"",
                "modifier must be followed by a key",
            ),
            (
                "[tui.keys.browse]\nx = \"dne\"\n",
                "tui.keys.browse.\"x\"",
                "did you mean \"done\"",
            ),
            (
                "[tui]\npreset = \"defaults\"\n",
                "tui.preset",
                "did you mean \"default\"",
            ),
        ];
        for (source, key, detail) in cases {
            let temp = TempDir::new();
            let path = temp.join("nib.toml");
            write(&path, source);
            let error = load(&temp.0, None, no_env).unwrap_err().to_string();
            assert!(error.contains(&path.display().to_string()), "{error}");
            assert!(error.contains(key), "{error}");
            assert!(error.contains(detail), "{error}");
        }
    }

    #[test]
    fn saved_views_merge_by_name_through_the_layers() {
        let temp = TempDir::new();
        let user = temp.join("user.toml");
        write(
            &user,
            "[tui.views]\nwork = \"list:work\"\nsoon = \"due:week\"\n",
        );
        let project = temp.join("project");
        fs::create_dir(&project).unwrap();
        write(
            &project.join("nib.toml"),
            "[tui.views]\nwork = \"list:work pri:high\"\n\"Done today\" = \"is:done due:today\"\n",
        );

        let config = load(&project, Some(&user), no_env).unwrap();
        assert_eq!(
            config.tui.views.into_iter().collect::<Vec<_>>(),
            [
                ("Done today".to_owned(), "is:done due:today".to_owned()),
                ("soon".to_owned(), "due:week".to_owned()),
                ("work".to_owned(), "list:work pri:high".to_owned()),
            ]
        );
    }

    #[test]
    fn invalid_saved_views_name_the_file_and_key() {
        let cases = [
            (
                "[tui.views]\nurgent = \"pri:urgent\"\n",
                "tui.views.\"urgent\"",
                "invalid value \"urgent\" for pri filter",
                (2, 11),
            ),
            (
                "[tui.views]\nurgent = 3\n",
                "tui.views.\"urgent\"",
                "must be a filter expression string",
                (2, 1),
            ),
            (
                "[tui.views]\n\" padded\" = \"due:today\"\n",
                "tui.views.\" padded\"",
                "surrounding spaces",
                (2, 2),
            ),
        ];
        for (source, key, detail, location) in cases {
            let temp = TempDir::new();
            let path = temp.join("nib.toml");
            write(&path, source);
            let error = load(&temp.0, None, no_env).unwrap_err();
            let text = error.to_string();
            assert!(text.starts_with("invalid saved view in "), "{text}");
            assert!(text.contains(&path.display().to_string()), "{text}");
            assert!(text.contains(key), "{text}");
            assert!(text.contains(detail), "{text}");
            let ConfigError::View { line, column, .. } = &error else {
                panic!("{error:?}");
            };
            assert_eq!((*line, *column), location, "{text}");
        }
    }

    #[test]
    fn duplicate_canonical_key_chords_are_rejected() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        write(
            &path,
            "[tui.keys.filter]\n\"ctrl-o\" = \"act-menu\"\n\"Ctrl-O\" = \"accept\"\n",
        );

        let error = load(&temp.0, None, no_env).unwrap_err().to_string();
        assert!(error.contains("duplicates"), "{error}");
        assert!(error.contains("ctrl-o"), "{error}");
        assert!(error.contains("Ctrl-O"), "{error}");
    }

    #[test]
    fn keymap_diagnostics_point_to_the_binding_value() {
        let temp = TempDir::new();
        write(&temp.join("nib.toml"), "[tui.keys.browse]\nx = \"dne\"\n");

        let error = load(&temp.0, None, no_env).unwrap_err();
        assert!(
            error.to_string().ends_with(
                "for tui.keys.browse.\"x\": unknown keymap action \"dne\"; did you mean \"done\"?"
            ),
            "{error}"
        );
        let ConfigError::Keymap { line, column, .. } = &error else {
            panic!("{error:?}");
        };
        assert_eq!((*line, *column), (2, 6), "{error:?}");
    }

    #[test]
    fn none_cannot_be_combined_with_other_actions() {
        let temp = TempDir::new();
        write(
            &temp.join("nib.toml"),
            "[tui.keys.browse]\nx = [\"none\", \"down\"]\n",
        );

        let error = load(&temp.0, None, no_env).unwrap_err().to_string();
        assert!(error.contains("must be the only action"), "{error}");
    }

    #[test]
    fn inspection_reports_loaded_files_and_parse_errors() {
        let temp = TempDir::new();
        let valid = temp.join("nib.toml");
        let invalid = temp.join(".nib.local.toml");
        write(&valid, "[task]\nlist = \"Work\"\n");
        write(&invalid, "[task]\ncolour = \"blue\"\n");

        let inspection = inspect(&temp.0, None, no_env);
        assert!(inspection.error().is_some());
        assert_eq!(inspection.files().len(), 2);
        assert_eq!(inspection.files()[0].path(), valid);
        assert_eq!(inspection.files()[0].error(), None);
        assert_eq!(inspection.files()[1].path(), invalid);
        assert!(inspection.files()[1].error().is_some());
    }

    #[test]
    fn nearer_files_override_farther_files_field_by_field() {
        let temp = TempDir::new();
        let project = temp.join("project");
        let package = project.join("package");
        fs::create_dir_all(&package).unwrap();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Root\"\npriority = \"low\"\nparse_inline = false\n[cal]\ncalendars = [\"Home\"]\nshow_calendar = true\n",
        );
        write(
            &project.join(".nib.toml"),
            "[task]\nlist = \"Project\"\n[note]\nfolder = \"Work\"\n",
        );
        write(&package.join("nib.toml"), "[task]\npriority = \"high\"\n");

        let config = load(&package, None, no_env).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Project"));
        assert_eq!(config.task.priority, Some(Priority::High));
        assert!(!config.task.parse_inline);
        assert_eq!(config.cal.calendars, ["Home"]);
        assert!(config.cal.show_calendar);
        assert_eq!(config.note.folder.as_deref(), Some("Work"));
    }

    #[test]
    fn local_overrides_project_file() {
        let temp = TempDir::new();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Project\"\npriority = \"low\"\n",
        );
        write(&temp.join(".nib.local.toml"), "[task]\nlist = \"Local\"\n");

        let entries = inspect(&temp.0, None, no_env).into_entries().unwrap();
        assert_eq!(
            entries[0],
            ConfigEntry {
                key: ConfigKey::TaskList,
                value: ConfigValue::String("Local".into()),
                origin: ConfigOrigin::File(temp.join(".nib.local.toml")),
            }
        );
        assert_eq!(entries[1].value, ConfigValue::Priority(Priority::Low));
        assert_eq!(entries[1].origin, ConfigOrigin::File(temp.join("nib.toml")));
    }

    #[test]
    fn hidden_project_file_fallback_has_the_file_origin() {
        let temp = TempDir::new();
        let hidden = temp.join(".nib.toml");
        write(&hidden, "[task]\nlist = \"Hidden\"\n");

        let entries = inspect(&temp.0, None, no_env).into_entries().unwrap();

        assert_eq!(entries[0].origin, ConfigOrigin::File(hidden));
        assert_eq!(entries[0].value, ConfigValue::String("Hidden".into()));
    }

    #[test]
    fn environment_overrides_files() {
        let temp = TempDir::new();
        write(
            &temp.join("nib.toml"),
            "[task]\nlist = \"Project\"\npriority = \"low\"\nparse_inline = false\n[cal]\ncalendars = [\"Home\"]\nshow_calendar = true\n[note]\nfolder = \"Archive\"\n",
        );
        let env = HashMap::from([
            ("NIB_TASK_LIST", "Environment"),
            ("NIB_TASK_PRIORITY", "medium"),
            ("NIB_TASK_PARSE_INLINE", "true"),
            ("NIB_CAL_CALENDARS", "Work, Birthdays"),
            ("NIB_CAL_SHOW_CALENDAR", "false"),
            ("NIB_NOTE_FOLDER", "Inbox"),
        ]);

        let config = load(&temp.0, None, |name| env.get(name).map(OsString::from)).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Environment"));
        assert_eq!(config.task.priority, Some(Priority::Medium));
        assert!(config.task.parse_inline);
        assert_eq!(config.cal.calendars, ["Work", "Birthdays"]);
        assert!(!config.cal.show_calendar);
        assert_eq!(config.note.folder.as_deref(), Some("Inbox"));
    }

    #[test]
    fn show_calendar_reports_its_origin() {
        let temp = TempDir::new();
        let project = temp.join("nib.toml");
        write(&project, "[cal]\nshow_calendar = true\n");

        let origin = |env: Option<&str>| {
            inspect(&temp.0, None, |name| {
                env.filter(|_| name == "NIB_CAL_SHOW_CALENDAR")
                    .map(OsString::from)
            })
            .into_entries()
            .unwrap()
            .into_iter()
            .find(|entry| entry.key == ConfigKey::CalShowCalendar)
            .unwrap()
        };

        assert_eq!(
            origin(None),
            ConfigEntry {
                key: ConfigKey::CalShowCalendar,
                value: ConfigValue::Bool(true),
                origin: ConfigOrigin::File(project),
            }
        );
        assert_eq!(
            origin(Some("false")),
            ConfigEntry {
                key: ConfigKey::CalShowCalendar,
                value: ConfigValue::Bool(false),
                origin: ConfigOrigin::Environment("NIB_CAL_SHOW_CALENDAR"),
            }
        );
    }

    #[test]
    fn show_calendar_can_be_set_and_unset() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");
        assert_eq!(
            key("cal.show_calendar").unwrap(),
            ConfigKey::CalShowCalendar
        );

        assert_eq!(
            set(&path, ConfigKey::CalShowCalendar, "true", &temp.locks())
                .unwrap()
                .value,
            ConfigValue::Bool(true)
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[cal]\nshow_calendar = true\n"
        );
        assert!(
            load(&temp.0, Some(&path), no_env)
                .unwrap()
                .cal
                .show_calendar
        );
        assert!(set(&path, ConfigKey::CalShowCalendar, "yes", &temp.locks()).is_err());

        assert!(
            unset(&path, ConfigKey::CalShowCalendar, &temp.locks())
                .unwrap()
                .value
        );
        assert!(
            !load(&temp.0, Some(&path), no_env)
                .unwrap()
                .cal
                .show_calendar
        );
    }

    #[test]
    fn tui_preset_can_be_set_and_overridden_by_the_environment() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");

        assert_eq!(
            set(&path, ConfigKey::TuiPreset, "default", &temp.locks())
                .unwrap()
                .value,
            ConfigValue::String("default".into())
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[tui]\npreset = \"default\"\n"
        );
        let inspection = inspect(&temp.0, Some(&path), |name| {
            (name == "NIB_TUI_PRESET").then(|| OsString::from("default"))
        });
        assert_eq!(
            inspection.origin(ConfigKey::TuiPreset),
            Some(&ConfigOrigin::Environment("NIB_TUI_PRESET"))
        );
        assert!(set(&path, ConfigKey::TuiPreset, "vim", &temp.locks()).is_err());
        let error = load(&temp.0, Some(&path), |name| {
            (name == "NIB_TUI_PRESET").then(|| OsString::from("vim"))
        })
        .unwrap_err()
        .to_string();
        assert!(error.contains("NIB_TUI_PRESET"), "{error}");
        assert!(error.contains("default"), "{error}");
    }

    #[test]
    fn origins_follow_field_by_field_precedence() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        let user = temp.join("user.toml");
        let project_file = project.join("nib.toml");
        write(&user, "[task]\nlist = \"User\"\npriority = \"low\"\n");
        write(
            &project_file,
            "[task]\nlist = \"Project\"\n[cal]\ncalendars = [\"Work\"]\n",
        );
        let inspection = inspect(&project, Some(&user), |name| {
            (name == "NIB_TASK_PRIORITY").then(|| OsString::from("high"))
        });
        let entries = inspection.into_entries().unwrap();

        assert_eq!(
            entries,
            vec![
                ConfigEntry {
                    key: ConfigKey::TaskList,
                    value: ConfigValue::String("Project".into()),
                    origin: ConfigOrigin::File(project_file.clone()),
                },
                ConfigEntry {
                    key: ConfigKey::TaskPriority,
                    value: ConfigValue::Priority(Priority::High),
                    origin: ConfigOrigin::Environment("NIB_TASK_PRIORITY"),
                },
                ConfigEntry {
                    key: ConfigKey::TaskParseInline,
                    value: ConfigValue::Bool(true),
                    origin: ConfigOrigin::Default,
                },
                ConfigEntry {
                    key: ConfigKey::CalCalendars,
                    value: ConfigValue::Strings(vec!["Work".into()]),
                    origin: ConfigOrigin::File(project_file),
                },
                ConfigEntry {
                    key: ConfigKey::CalShowCalendar,
                    value: ConfigValue::Bool(false),
                    origin: ConfigOrigin::Default,
                },
                ConfigEntry {
                    key: ConfigKey::NoteFolder,
                    value: ConfigValue::Unset,
                    origin: ConfigOrigin::Default,
                },
                ConfigEntry {
                    key: ConfigKey::TuiPreset,
                    value: ConfigValue::String("default".into()),
                    origin: ConfigOrigin::Default,
                },
            ]
        );
    }

    #[test]
    fn flags_override_environment_when_actions_are_built() {
        let env = HashMap::from([
            ("NIB_TASK_LIST", "Environment"),
            ("NIB_TASK_PRIORITY", "low"),
            ("NIB_CAL_CALENDARS", "Home"),
        ]);
        let config = load(Path::new("/"), None, |name| {
            env.get(name).map(OsString::from)
        })
        .unwrap();
        let now = date(2026, 10, 26)
            .at(9, 0, 0, 0)
            .to_zoned(TimeZone::fixed(offset(-4)))
            .unwrap();

        let task = Cli::try_parse_from([
            "nib",
            "t",
            "Review",
            "PR",
            "--list",
            "Flag",
            "--priority",
            "high",
        ])
        .unwrap()
        .command
        .into_action(&now, &config)
        .unwrap()
        .unwrap();
        assert!(matches!(
            task,
            Action::AddTask(NewReminder { list: Some(list), priority: Some(Priority::High), .. })
                if list == "Flag"
        ));

        let events = Cli::try_parse_from([
            "nib",
            "cal",
            "--calendar",
            "Work",
            "--calendar",
            "Birthdays",
        ])
        .unwrap()
        .command
        .into_action(&now, &config)
        .unwrap()
        .unwrap();
        assert!(matches!(
            events,
            Action::ListEvents { query: EventQuery { calendars, .. }, .. }
                if calendars == ["Work", "Birthdays"]
        ));
    }

    #[test]
    fn user_config_is_not_read_again_as_a_project_file() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        write(&temp.join("nib.toml"), "[task]\nlist = \"Project\"\n");
        let user = project.join("nib.toml");
        write(&user, "[task]\nlist = \"User\"\n");
        write(&project.join(".nib.toml"), "[task]\npriority = \"high\"\n");

        let config = load(&project, Some(&user), no_env).unwrap();
        assert_eq!(config.task.list.as_deref(), Some("Project"));
        assert_eq!(config.task.priority, Some(Priority::High));
    }

    #[test]
    fn unknown_keys_name_the_file_and_location() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        write(&path, "[task]\ncolour = \"blue\"\n");

        let inspection = inspect(&temp.0, None, no_env);
        let error = inspection.error().unwrap().to_string();
        assert!(error.contains(&path.display().to_string()), "{error}");
        assert!(!error.contains("line 2, column 1"), "{error}");
        assert!(error.contains("unknown field `colour`"), "{error}");
        assert!(
            inspection.files()[0]
                .error()
                .unwrap()
                .contains("line 2, column 1")
        );
    }

    #[test]
    fn file_errors_render_the_source_span() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        write(&path, "[task]\ncolour = \"blue\"\n");

        let error = load(&temp.0, None, no_env).unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("colour = \"blue\""), "{rendered}");
        assert!(rendered.contains("invalid configuration"), "{rendered}");
        assert!(rendered.contains(&path.display().to_string()), "{rendered}");
    }

    #[test]
    fn invalid_file_values_render_the_source_span() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        write(&path, "[task]\npriority = \"urgent\"\n");

        let error = load(&temp.0, None, no_env).unwrap_err();
        let rendered = format!("{:?}", miette::Report::new(error));
        assert!(rendered.contains("priority = \"urgent\""), "{rendered}");
        assert!(rendered.contains("invalid configuration"), "{rendered}");
        assert!(rendered.contains(&path.display().to_string()), "{rendered}");
    }

    #[test]
    fn rendered_diagnostics_are_stable_for_unicode_and_file_precedence() {
        let temp = TempDir::new();
        let project = temp.join("project");
        fs::create_dir_all(&project).unwrap();
        let far = temp.join("nib.toml");
        let near = project.join(".nib.local.toml");

        write(&far, "[task]\nlist = \"日本語\", priority = \"x\"\n");
        let unicode = render(inspect(&project, None, no_env).error().unwrap());

        write(&far, "[task]\nlist = \"Far\"\n");
        write(&near, "[note]\nfolder = 42\n");
        let nearer = render(inspect(&project, None, no_env).error().unwrap());

        write(&far, "[task]\npriority = \"urgent\"\n");
        fs::remove_file(&near).unwrap();
        let farther = render(inspect(&project, None, no_env).error().unwrap());
        let rendered = format!("UNICODE\n{unicode}\nNEARER\n{nearer}\nFARTHER\n{farther}")
            .replace(&temp.0.display().to_string(), "$ROOT");

        assert!(!rendered.contains(" at line "), "{rendered}");
        insta::assert_snapshot!(rendered, @r#"
        UNICODE
        nib::config::invalid_file

          × invalid configuration in $ROOT/nib.toml: unexpected key or value, expected newline, `#`
           ╭─[$ROOT/nib.toml:2:19]
         1 │ [task]
         2 │ list = "日本語", priority = "x"
           ·                ▲
           ·                ╰── unexpected key or value, expected newline, `#`
           ╰────

        NEARER
        nib::config::invalid_file

          × invalid configuration in $ROOT/project/.nib.local.toml: invalid type: integer `42`, expected a string
           ╭─[$ROOT/project/.nib.local.toml:2:10]
         1 │ [note]
         2 │ folder = 42
           ·          ─┬
           ·           ╰── invalid type: integer `42`, expected a string
           ╰────

        FARTHER
        nib::config::invalid_file

          × invalid configuration in $ROOT/nib.toml: unknown variant `urgent`, expected one of `low`, `medium`, `high`
           ╭─[$ROOT/nib.toml:2:12]
         1 │ [task]
         2 │ priority = "urgent"
           ·            ────┬───
           ·                ╰── unknown variant `urgent`, expected one of `low`, `medium`, `high`
           ╰────
        "#);
    }

    #[test]
    fn set_and_unset_preserve_unrelated_formatting() {
        let temp = TempDir::new();
        let path = temp.join("nested/config.toml");
        write(
            &path,
            "# heading\n[task]\nlist    = \"Inbox\" # keep me\nparse_inline = false\n",
        );

        set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap();
        let edited = fs::read_to_string(&path).unwrap();
        assert!(edited.contains("# heading"), "{edited}");
        assert!(edited.contains("list    = \"Work\" # keep me"), "{edited}");
        assert!(edited.contains("parse_inline = false"), "{edited}");

        assert!(
            unset(&path, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );
        let edited = fs::read_to_string(&path).unwrap();
        assert!(!edited.contains("list"), "{edited}");
        assert!(edited.contains("parse_inline = false"), "{edited}");
    }

    #[test]
    fn set_creates_parent_directories_and_validates_values() {
        let temp = TempDir::new();
        let path = temp.join("one/two/config.toml");
        assert_eq!(
            set(
                &path,
                ConfigKey::CalCalendars,
                "[\"Work\", \"Home\"]",
                &temp.locks()
            )
            .unwrap()
            .value,
            ConfigValue::Strings(vec!["Work".into(), "Home".into()])
        );
        assert_eq!(
            load(&temp.0, Some(&path), no_env).unwrap().cal.calendars,
            ["Work", "Home"]
        );
        assert!(set(&path, ConfigKey::TaskPriority, "urgent", &temp.locks()).is_err());
        assert!(set(&path, ConfigKey::TaskParseInline, "yes", &temp.locks()).is_err());
        assert_eq!(
            set(&path, ConfigKey::CalCalendars, "Work, Home", &temp.locks())
                .unwrap()
                .value,
            ConfigValue::Strings(vec!["Work".into(), "Home".into()])
        );
        assert!(
            set(
                &path,
                ConfigKey::CalCalendars,
                "[\"Work\"] trailing",
                &temp.locks()
            )
            .is_err()
        );
        assert!(
            set(
                &path,
                ConfigKey::CalCalendars,
                "[\"Work\", 3]",
                &temp.locks()
            )
            .is_err()
        );
        let error = set(&path, ConfigKey::CalCalendars, "[", &temp.locks())
            .unwrap_err()
            .to_string();
        assert!(error.contains("Work, Personal"), "{error}");
        assert!(error.contains("[\"Work\", \"Personal\"]"), "{error}");
    }

    #[test]
    fn set_and_unset_support_inline_tables_and_dotted_keys() {
        let temp = TempDir::new();
        let inline = temp.join("inline.toml");
        write(&inline, "task = { list = \"Inbox\" }\n");

        set(&inline, ConfigKey::TaskPriority, "high", &temp.locks()).unwrap();
        assert_eq!(
            load(&temp.0, Some(&inline), no_env).unwrap().task.priority,
            Some(Priority::High)
        );
        assert!(
            unset(&inline, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );
        assert!(!fs::read_to_string(&inline).unwrap().contains("list"));
        assert!(
            unset(&inline, ConfigKey::TaskPriority, &temp.locks())
                .unwrap()
                .value
        );
        assert!(!fs::read_to_string(&inline).unwrap().contains("task"));

        let dotted = temp.join("dotted.toml");
        write(&dotted, "task.list = \"Inbox\"\n");
        set(&dotted, ConfigKey::TaskPriority, "medium", &temp.locks()).unwrap();
        assert!(
            unset(&dotted, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );
        let edited = fs::read_to_string(&dotted).unwrap();
        assert!(edited.contains("task.priority = \"medium\""), "{edited}");
    }

    #[test]
    fn unset_keeps_comment_bearing_empty_sections() {
        let temp = TempDir::new();
        let section = temp.join("section.toml");
        let comments = "#:schema https://example.com/config.schema.json\n# Header comment\n";
        write(&section, &format!("{comments}[note]\nfolder = \"Notes\"\n"));

        assert!(
            unset(&section, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );
        assert_eq!(
            fs::read_to_string(&section).unwrap(),
            format!("{comments}[note]\n")
        );

        let header_suffix = temp.join("header-suffix.toml");
        write(
            &header_suffix,
            "[note] # Header suffix\nfolder = \"Notes\"\n",
        );

        assert!(
            unset(&header_suffix, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );
        assert_eq!(
            fs::read_to_string(&header_suffix).unwrap(),
            "[note] # Header suffix\n"
        );

        let inline = temp.join("inline-comments.toml");
        write(&inline, "task = { priority = \"high\" } # mine\n");

        assert!(
            unset(&inline, ConfigKey::TaskPriority, &temp.locks())
                .unwrap()
                .value
        );
        assert_eq!(fs::read_to_string(&inline).unwrap(), "task = {} # mine\n");
    }

    #[test]
    fn config_edits_fail_clearly_while_another_command_holds_the_lock() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");
        write(&path, "[task]\nlist = \"Inbox\"\n");
        let held = crate::infra::host::config_lock(&path, &temp.locks()).unwrap();

        let error = set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap_err();

        assert_eq!(
            error.to_string(),
            format!(
                "another nib config command is changing {}; try again when it finishes",
                fs::canonicalize(&path).unwrap().display()
            )
        );
        let mut rendered = String::new();
        GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
            .render_report(&mut rendered, &error)
            .unwrap();
        assert!(!rendered.contains('▶'), "{rendered}");
        assert!(matches!(
            unset(&path, ConfigKey::TaskList, &temp.locks()),
            Err(ConfigCommandError::Locked { .. })
        ));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[task]\nlist = \"Inbox\"\n"
        );

        drop(held);
        set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap();
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[task]\nlist = \"Work\"\n"
        );
    }

    #[test]
    fn config_edits_ignore_lock_files_from_older_versions() {
        let temp = TempDir::new();
        let path = temp.join("nib.toml");
        let legacy = temp.join("nib.toml.lock");
        write(&path, "[task]\nlist = \"Inbox\"\n");
        write(&legacy, "");

        set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap();
        assert!(
            unset(&path, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert!(legacy.exists());
        assert_eq!(fs::read_dir(temp.locks()).unwrap().count(), 1);
    }

    #[test]
    fn config_lock_failures_name_the_lock_folder() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");
        write(&path, "[task]\nlist = \"Inbox\"\n");
        write(&temp.locks(), "");

        let error = set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap_err();

        assert!(
            matches!(&error, ConfigCommandError::Lock { path, .. } if *path == temp.locks()),
            "{error:?}"
        );
        assert_eq!(
            error.to_string(),
            format!(
                "couldn't lock configuration with {}",
                temp.locks().display()
            )
        );
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "[task]\nlist = \"Inbox\"\n"
        );
    }

    #[test]
    fn config_lock_failures_render_as_a_diagnostic() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");
        write(&temp.locks(), "");

        let error = set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap_err();
        let mut rendered = String::new();
        GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor())
            .with_width(usize::MAX)
            .render_report(&mut rendered, &error)
            .unwrap();

        insta::assert_snapshot!(rendered.replace(&temp.0.display().to_string(), "$ROOT"), @r"
        nib::config::lock

          × couldn't lock configuration with $ROOT/locks
          ╰─▶ File exists (os error 17)
        ");
    }

    #[test]
    fn legacy_config_lock_files_report_leftovers_for_each_target() {
        let temp = TempDir::new();
        let user = temp.join("user/config.toml");
        let project = temp.join("project");
        let user_lock = temp.join("user/config.toml.lock");
        let local_lock = project.join(".nib.local.toml.lock");
        write(&user_lock, "");
        write(&local_lock, "");

        let locks = inspect(&project, Some(&user), no_env).legacy_config_lock_files();

        assert_eq!(locks, [user_lock, local_lock]);
        assert_eq!(
            inspect(&temp.0, Some(&temp.join("absent.toml")), no_env).legacy_config_lock_files(),
            Vec::<PathBuf>::new()
        );
    }

    #[test]
    fn unset_moves_comments_from_inline_tables_to_the_next_item() {
        let temp = TempDir::new();
        let path = temp.join("inline.toml");
        write(
            &path,
            "# Inbox\ntask = { list = \"Inbox\" }\n\n[note]\nfolder = \"Notes\"\n",
        );

        assert!(
            unset(&path, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# Inbox\n\n[note]\nfolder = \"Notes\"\n"
        );
    }

    #[test]
    fn unset_moves_comments_from_dotted_keys_to_the_next_root_key() {
        let temp = TempDir::new();
        let path = temp.join("dotted.toml");
        write(
            &path,
            "# Inbox\ntask.list = \"Inbox\"\nnote.folder = \"Notes\"\n",
        );

        assert!(
            unset(&path, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# Inbox\nnote.folder = \"Notes\"\n"
        );
    }

    #[test]
    fn unset_moves_comments_to_the_end_of_the_file_when_no_item_follows() {
        let temp = TempDir::new();
        let inline = temp.join("inline.toml");
        write(&inline, "# Inbox\ntask = { list = \"Inbox\" }\n");

        assert!(
            unset(&inline, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(fs::read_to_string(inline).unwrap(), "# Inbox\n");

        let schema = temp.join("schema.toml");
        write(
            &schema,
            "#:schema https://example.com/config.schema.json\ntask = { list = \"Inbox\" }\n",
        );

        assert!(
            unset(&schema, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(schema).unwrap(),
            "#:schema https://example.com/config.schema.json\n"
        );

        let dotted = temp.join("dotted.toml");
        write(&dotted, "# Inbox\ntask.list = \"Inbox\"\n");

        assert!(
            unset(&dotted, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(fs::read_to_string(dotted).unwrap(), "# Inbox\n");
    }

    #[test]
    fn unset_separates_moved_comments_from_an_eof_comment_without_newline() {
        let temp = TempDir::new();
        let path = temp.join("eof.toml");
        write(
            &path,
            "[task]\nlist = \"Inbox\"\n[note]\n# folder doc\nfolder = \"Notes\"\n# footer",
        );

        assert!(
            unset(&path, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "[task]\nlist = \"Inbox\"\n# footer\n# folder doc\n"
        );
    }

    #[test]
    fn unset_keeps_removed_key_comments_when_the_section_stays() {
        let temp = TempDir::new();
        let path = temp.join("section-comment.toml");
        write(&path, "# Header\n[note]\n# folder doc\nfolder = \"x\"\n");

        assert!(
            unset(&path, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "# Header\n[note]\n# folder doc\n"
        );

        let followed = temp.join("section-comment-followed.toml");
        write(
            &followed,
            "# Header\n[note]\n# folder doc\nfolder = \"x\"\n\n[task]\nlist = \"Inbox\"\n",
        );

        assert!(
            unset(&followed, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(followed).unwrap(),
            "# Header\n[note]\n# folder doc\n\n[task]\nlist = \"Inbox\"\n"
        );
    }

    #[test]
    fn set_after_unsetting_a_kept_section_key_places_the_moved_comment_below_the_key() {
        let temp = TempDir::new();
        let path = temp.join("unset-then-set.toml");
        write(&path, "# Header\n[note]\n# folder doc\nfolder = \"x\"\n");

        assert!(
            unset(&path, ConfigKey::NoteFolder, &temp.locks())
                .unwrap()
                .value
        );
        set(&path, ConfigKey::NoteFolder, "Notes", &temp.locks()).unwrap();

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# Header\n[note]\nfolder = \"Notes\"\n# folder doc\n"
        );
    }

    #[test]
    fn unset_moves_key_comments_from_a_plain_section_to_the_next_item() {
        let temp = TempDir::new();
        let path = temp.join("key-comment.toml");
        write(
            &path,
            "[task]\n# doc\nlist = \"Inbox\"\n\n[note]\nfolder = \"Notes\"\n",
        );

        assert!(
            unset(&path, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "# doc\n\n[note]\nfolder = \"Notes\"\n"
        );
    }

    #[test]
    fn unset_missing_key_does_not_create_or_rewrite_anything() {
        let temp = TempDir::new();
        let missing = temp.join("missing/config.toml");
        assert!(
            !unset(&missing, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );
        assert!(!missing.parent().unwrap().exists());

        let existing = temp.join("config.toml");
        let original = "[note]\nfolder = \"Notes\"\n";
        write(&existing, original);
        assert!(
            !unset(&existing, ConfigKey::TaskList, &temp.locks())
                .unwrap()
                .value
        );
        assert_eq!(fs::read_to_string(existing).unwrap(), original);
    }

    #[test]
    fn edits_preserve_crlf_and_literal_string_quotes() {
        let temp = TempDir::new();
        let path = temp.join("config.toml");
        write(&path, "[task]\r\nlist = 'Inbox'\r\n");

        set(&path, ConfigKey::TaskList, "Work", &temp.locks()).unwrap();

        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "[task]\r\nlist = 'Work'\r\n"
        );
    }

    #[test]
    fn schema_matches_file_types_and_config_keys() {
        let schema: serde_json::Value =
            serde_json::from_str(include_str!("../../docs/config.schema.json")).unwrap();
        let mut schema_keys = BTreeSet::new();
        let mut sample = toml::map::Map::new();
        for (section, section_schema) in schema["properties"].as_object().unwrap() {
            let properties = section_schema["properties"].as_object().unwrap();
            let mut section_sample = toml::map::Map::new();
            for (name, property) in properties {
                if name == "views" {
                    let mut views = toml::map::Map::new();
                    views.insert("today".into(), toml::Value::String("due:today".into()));
                    section_sample.insert(name.clone(), toml::Value::Table(views));
                    continue;
                }
                if name == "keys" {
                    let mut browse = toml::map::Map::new();
                    browse.insert("x".into(), toml::Value::String("done".into()));
                    let mut states = toml::map::Map::new();
                    states.insert("browse".into(), toml::Value::Table(browse));
                    section_sample.insert(name.clone(), toml::Value::Table(states));
                    continue;
                }
                schema_keys.insert(format!("{section}.{name}"));
                let value = if let Some(first) = property["enum"].as_array().and_then(|v| v.first())
                {
                    toml::Value::String(first.as_str().unwrap().into())
                } else {
                    match property["type"].as_str().unwrap() {
                        "string" => toml::Value::String("x".into()),
                        "boolean" => toml::Value::Boolean(true),
                        "array" => toml::Value::Array(vec![toml::Value::String("x".into())]),
                        value_type => panic!("unsupported schema sample type {value_type}"),
                    }
                };
                section_sample.insert(name.clone(), value);
            }
            sample.insert(section.clone(), toml::Value::Table(section_sample));
        }
        let file: FileConfig = toml::Value::Table(sample).try_into().unwrap();
        let FileConfig {
            task,
            cal,
            note,
            tui,
        } = file;
        let FileTask {
            list,
            priority,
            parse_inline,
        } = task.unwrap();
        assert!(list.is_some() && priority.is_some() && parse_inline.is_some());
        let FileCal {
            calendars,
            show_calendar,
        } = cal.unwrap();
        assert!(calendars.is_some() && show_calendar.is_some());
        let FileNote { folder } = note.unwrap();
        assert!(folder.is_some());
        let FileTui {
            preset,
            keys,
            views,
        } = tui.unwrap();
        assert!(preset.is_some() && keys.is_some() && views.is_some());

        let supported = ConfigKey::ALL
            .map(ConfigKey::as_str)
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();
        assert_eq!(schema_keys, supported);
        for key in ConfigKey::ALL {
            assert_eq!(ConfigKey::parse(key.as_str()), Some(key));
        }
        let schema_actions = schema["$defs"]["action"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<BTreeSet<_>>();
        let supported_actions = KeymapAction::ALL
            .map(KeymapAction::as_str)
            .into_iter()
            .collect::<BTreeSet<_>>();
        assert_eq!(schema_actions, supported_actions);
    }

    #[test]
    fn invalid_environment_values_name_the_variable() {
        let cases = [
            ("NIB_TASK_PRIORITY", "urgent"),
            ("NIB_TASK_PARSE_INLINE", "sometimes"),
            ("NIB_CAL_SHOW_CALENDAR", "yes"),
            ("NIB_TUI_PRESET", "vim"),
        ];
        for (variable, value) in cases {
            let error = load(Path::new("/"), None, |name| {
                (name == variable).then(|| OsString::from(value))
            })
            .unwrap_err()
            .to_string();
            assert!(error.contains(variable), "{error}");
            assert!(error.contains(value), "{error}");

            let diagnostic = load(Path::new("/"), None, |name| {
                (name == variable).then(|| OsString::from(value))
            })
            .unwrap_err();
            let rendered = format!("{:?}", miette::Report::new(diagnostic));
            assert!(rendered.contains("nib doctor"), "{rendered}");
        }
    }
}
