use std::ffi::OsString;
use std::process::Command;

use crate::domain::{
    AutomationStatus, DoctorProbe, EditorSetting, PermissionStatus, terminal_name,
};

pub struct SystemDoctor {
    term_program: Option<String>,
    visual: Option<String>,
    editor: Option<String>,
}

impl SystemDoctor {
    pub fn new(
        term_program: Option<OsString>,
        visual: Option<OsString>,
        editor: Option<OsString>,
    ) -> Self {
        Self {
            term_program: nonempty(term_program),
            visual: nonempty(visual),
            editor: nonempty(editor),
        }
    }
}

impl DoctorProbe for SystemDoctor {
    fn macos_version(&self) -> Result<String, String> {
        let output = Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .map_err(|error| format!("couldn't run sw_vers: {error}"))?;
        if !output.status.success() {
            return Err(format!("sw_vers exited with {}", output.status));
        }
        String::from_utf8(output.stdout)
            .map(|version| version.trim().to_owned())
            .map_err(|_| "sw_vers returned text that isn't valid UTF-8".into())
    }

    fn terminal(&self) -> (String, bool) {
        (
            terminal_name(self.term_program.as_deref()),
            self.term_program.is_some(),
        )
    }

    fn reminders_authorization(&self) -> PermissionStatus {
        authorization(Entity::Reminders)
    }

    fn calendars_authorization(&self) -> PermissionStatus {
        authorization(Entity::Calendars)
    }

    fn notes_automation(&self) -> AutomationStatus {
        notes_automation()
    }

    fn editor(&self) -> EditorSetting {
        if let Some(visual) = &self.visual {
            EditorSetting {
                variable: Some("VISUAL"),
                command: Some(visual.clone()),
            }
        } else if let Some(editor) = &self.editor {
            EditorSetting {
                variable: Some("EDITOR"),
                command: Some(editor.clone()),
            }
        } else {
            EditorSetting {
                variable: None,
                command: None,
            }
        }
    }
}

fn nonempty(value: Option<OsString>) -> Option<String> {
    value
        .and_then(|value| value.into_string().ok())
        .filter(|value| !value.trim().is_empty())
}

#[derive(Clone, Copy)]
enum Entity {
    Reminders,
    Calendars,
}

#[cfg(target_os = "macos")]
fn authorization(entity: Entity) -> PermissionStatus {
    match entity {
        Entity::Reminders => super::eventkit::reminders_authorization_status(),
        Entity::Calendars => super::eventkit::calendars_authorization_status(),
    }
}

#[cfg(not(target_os = "macos"))]
fn authorization(_: Entity) -> PermissionStatus {
    PermissionStatus::Unknown("nib doctor only supports macOS".into())
}

#[cfg(target_os = "macos")]
fn notes_automation() -> AutomationStatus {
    super::eventkit::notes_automation_status()
}

#[cfg(not(target_os = "macos"))]
fn notes_automation() -> AutomationStatus {
    AutomationStatus::Unknown("nib doctor only supports macOS".into())
}
