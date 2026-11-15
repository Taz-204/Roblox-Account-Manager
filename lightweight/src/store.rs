//! Account storage, shared by the window and the command line.
//!
//! Standard library only. Cookies are never stored in plain text: each account
//! keeps only the DPAPI-encrypted blob produced by the C# helper.

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub struct Account {
    pub name: String,
    pub note: String,
    /// Encrypted cookie from the helper's `protect` command. Empty if none saved.
    pub cookie_blob: String,
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

/// Removes tabs and line breaks so a field can't break the file format.
pub fn clean_field(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c == '\t' || c == '\n' || c == '\r' {
                ' '
            } else {
                c
            }
        })
        .collect::<String>()
        .trim()
        .to_string()
}

/// File format: one account per line, `name<TAB>note<TAB>cookie_blob`.
pub fn parse_accounts(text: &str) -> Vec<Account> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.splitn(3, '\t');
            Account {
                name: parts.next().unwrap_or("").trim().to_string(),
                note: parts.next().unwrap_or("").trim().to_string(),
                cookie_blob: parts.next().unwrap_or("").trim().to_string(),
            }
        })
        .filter(|a| !a.name.is_empty())
        .collect()
}

pub fn format_accounts(accounts: &[Account]) -> String {
    accounts
        .iter()
        .map(|a| {
            format!(
                "{}\t{}\t{}\n",
                clean_field(&a.name),
                clean_field(&a.note),
                a.cookie_blob.trim()
            )
        })
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

    fn account(name: &str, note: &str, blob: &str) -> Account {
        Account {
            name: name.into(),
            note: note.into(),
            cookie_blob: blob.into(),
        }
    }

    #[test]
    fn round_trips_accounts() {
        let accounts = vec![
            account("alt1", "farming bank", "QmxvYg=="),
            account("alt2", "", ""),
        ];
        let text = format_accounts(&accounts);
        assert_eq!(parse_accounts(&text), accounts);
    }

    #[test]
    fn skips_blank_and_nameless_lines() {
        let parsed = parse_accounts("\n\n\tno name\nreal\tnote\n");
        assert_eq!(parsed, vec![account("real", "note", "")]);
    }

    #[test]
    fn tabs_in_notes_cannot_corrupt_the_file() {
        let accounts = vec![account("alt", "has\ttab\nand newline", "BLOB")];
        let parsed = parse_accounts(&format_accounts(&accounts));
        assert_eq!(parsed, vec![account("alt", "has tab and newline", "BLOB")]);
    }

    #[test]
    fn clean_field_removes_separators() {
        assert_eq!(clean_field("  a\tb\r\nc  "), "a b  c");
    }
}
