use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

pub trait Host {
    fn edit(&mut self, path: &Path) -> io::Result<Duration>;
    fn ask(&mut self, prompt: &str) -> io::Result<String>;
    fn say(&mut self, message: &str);
    fn save_backup(&mut self, name: &str, contents: &str) -> io::Result<PathBuf>;
}

pub struct System;

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
        let path = dir.join(name);
        std::fs::write(&path, contents)?;
        Ok(path)
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
            edits: Default::default(),
            answers: Default::default(),
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
