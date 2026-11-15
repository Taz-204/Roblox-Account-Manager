//! Account storage and launching, shared by the window and the command line.
//!
//! Standard library only. Cookies and passwords are deliberately NOT stored.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    pub note: String,
}

pub fn config_path() -> PathBuf {
    let base = if cfg!(windows) {
        env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config"))
            .unwrap_or_else(|_| PathBuf::from("."))
    };
    base.join("rbx-alt-manager").join("accounts.txt")
}

/// File format: one account per line, `name<TAB>note`.
pub fn parse_accounts(text: &str) -> Vec<Account> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.splitn(2, '\t');
            Account {
                name: parts.next().unwrap_or("").trim().to_string(),
                note: parts.next().unwrap_or("").trim().to_string(),
            }
        })
        .filter(|a| !a.name.is_empty())
        .collect()
}

pub fn format_accounts(accounts: &[Account]) -> String {
    accounts
        .iter()
        .map(|a| format!("{}\t{}\n", a.name, a.note))
        .collect()
}

pub fn load(path: &Path) -> io::Result<Vec<Account>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(parse_accounts(&text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

pub fn save(path: &Path, accounts: &[Account]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, format_accounts(accounts))
}

/// Opens a Roblox experience by place ID using the roblox:// URI scheme.
/// This opens the client for whichever account is signed in by default.
pub fn open_place(place_id: u64) -> io::Result<()> {
    let url = format!("roblox://placeId={place_id}");
    let status = if cfg!(windows) {
        Command::new("cmd")
            .args(["/C", "start", "", &url])
            .status()?
    } else if cfg!(target_os = "macos") {
        Command::new("open").arg(&url).status()?
    } else {
        Command::new("xdg-open").arg(&url).status()?
    };
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("opener exited with {status}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_accounts() {
        let accounts = vec![
            Account {
                name: "alt1".into(),
                note: "farming bank".into(),
            },
            Account {
                name: "alt2".into(),
                note: String::new(),
            },
        ];
        let text = format_accounts(&accounts);
        assert_eq!(parse_accounts(&text), accounts);
    }

    #[test]
    fn skips_blank_and_nameless_lines() {
        let parsed = parse_accounts("\n\n\tno name\nreal\tnote\n");
        assert_eq!(
            parsed,
            vec![Account {
                name: "real".into(),
                note: "note".into()
            }]
        );
    }
}
