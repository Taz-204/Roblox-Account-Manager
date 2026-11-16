//! Storage for accounts, presets, and settings, shared by the window and the command line.
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
    /// Encrypted cookie from the helper. Empty if none saved.
    pub cookie_blob: String,
    /// Roblox user ID, used for the avatar. Empty if unknown.
    pub user_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Preset {
    pub name: String,
    pub place_id: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub multi_instance: bool,
    /// 0 means no cap.
    pub fps_cap: u32,
    /// Pause between launches when several accounts start together.
    pub launch_delay_ms: u64,
    /// Graphics quality override, 1 (lowest) to 10. 0 leaves Roblox's setting alone.
    pub graphics_level: u32,
    /// After launching, set running windows' priority and spread them across cores.
    pub cpu_share_on_launch: bool,
    /// Use Normal priority instead of Below Normal when sharing CPU.
    pub cpu_priority_normal: bool,
    /// RAM cap for each Roblox client, in MB. 0 means off.
    pub ram_limit_mb: u32,
    /// CPU cap for each Roblox client, as a percent of all cores. 0 means off.
    pub cpu_limit_percent: u32,
    /// Dark theme when true, light theme when false.
    pub dark_mode: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            multi_instance: false,
            fps_cap: 0,
            launch_delay_ms: 2000,
            graphics_level: 0,
            cpu_share_on_launch: false,
            cpu_priority_normal: false,
            ram_limit_mb: 0,
            cpu_limit_percent: 0,
            dark_mode: true,
        }
    }
}

fn config_dir() -> PathBuf {
    let base = if cfg!(windows) {
        env::var("APPDATA")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."))
    } else {
        env::var("HOME")
            .map(|h| PathBuf::from(h).join(".config"))
            .unwrap_or_else(|_| PathBuf::from("."))
    };
    base.join("rbx-alt-manager")
}

pub fn config_path() -> PathBuf {
    config_dir().join("accounts.txt")
}

pub fn presets_path() -> PathBuf {
    config_dir().join("presets.txt")
}

pub fn flags_path() -> PathBuf {
    config_dir().join("flags.json")
}

pub fn settings_path() -> PathBuf {
    config_dir().join("settings.txt")
}

/// Where an account's headshot is saved. None if the user ID isn't a plain number.
pub fn avatar_path(user_id: &str) -> Option<PathBuf> {
    if user_id.is_empty() || !user_id.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    Some(config_dir().join("avatars").join(format!("{user_id}.png")))
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

/// File format: one account per line, `name<TAB>note<TAB>cookie_blob<TAB>user_id`.
pub fn parse_accounts(text: &str) -> Vec<Account> {
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            let mut parts = l.splitn(4, '\t');
            Account {
                name: parts.next().unwrap_or("").trim().to_string(),
                note: parts.next().unwrap_or("").trim().to_string(),
                cookie_blob: parts.next().unwrap_or("").trim().to_string(),
                user_id: parts.next().unwrap_or("").trim().to_string(),
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
                "{}\t{}\t{}\t{}\n",
                clean_field(&a.name),
                clean_field(&a.note),
                a.cookie_blob.trim(),
                clean_field(&a.user_id)
            )
        })
        .collect()
}

/// File format: one preset per line, `name<TAB>place_id`.
pub fn parse_presets(text: &str) -> Vec<Preset> {
    text.lines()
        .filter_map(|l| {
            let mut parts = l.splitn(2, '\t');
            let name = clean_field(parts.next().unwrap_or(""));
            let place_id = clean_field(parts.next().unwrap_or(""));
            if name.is_empty() || place_id.is_empty() {
                None
            } else {
                Some(Preset { name, place_id })
            }
        })
        .collect()
}

pub fn format_presets(presets: &[Preset]) -> String {
    presets
        .iter()
        .map(|p| format!("{}\t{}\n", clean_field(&p.name), clean_field(&p.place_id)))
        .collect()
}

/// File format: `key=value` lines. Unknown keys are ignored.
pub fn parse_settings(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "multi_instance" => settings.multi_instance = value.trim() == "true",
            "fps_cap" => settings.fps_cap = value.trim().parse().unwrap_or(0),
            "graphics_level" => settings.graphics_level = value.trim().parse().unwrap_or(0),
            "cpu_share_on_launch" => settings.cpu_share_on_launch = value.trim() == "true",
            "cpu_priority_normal" => settings.cpu_priority_normal = value.trim() == "true",
            "ram_limit_mb" => settings.ram_limit_mb = value.trim().parse().unwrap_or(0),
            "cpu_limit_percent" => settings.cpu_limit_percent = value.trim().parse().unwrap_or(0),
            "dark_mode" => settings.dark_mode = value.trim() != "false",
            "launch_delay_ms" => {
                settings.launch_delay_ms = value
                    .trim()
                    .parse()
                    .unwrap_or(Settings::default().launch_delay_ms)
            }
            _ => {}
        }
    }
    settings
}

pub fn format_settings(settings: &Settings) -> String {
    format!(
        "multi_instance={}\nfps_cap={}\nlaunch_delay_ms={}\ngraphics_level={}\ncpu_share_on_launch={}\ncpu_priority_normal={}\nram_limit_mb={}\ncpu_limit_percent={}\ndark_mode={}\n",
        settings.multi_instance,
        settings.fps_cap,
        settings.launch_delay_ms,
        settings.graphics_level,
        settings.cpu_share_on_launch,
        settings.cpu_priority_normal,
        settings.ram_limit_mb,
        settings.cpu_limit_percent,
        settings.dark_mode
    )
}

fn read_or_default(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

fn write_file(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(path, text)
}

/// Reads a text file. A missing file is an error, so callers can use unwrap_or_default.
pub fn read_text(path: &Path) -> io::Result<String> {
    fs::read_to_string(path)
}

pub fn write_text(path: &Path, text: &str) -> io::Result<()> {
    write_file(path, text)
}

pub fn load(path: &Path) -> io::Result<Vec<Account>> {
    Ok(read_or_default(path)?
        .map(|t| parse_accounts(&t))
        .unwrap_or_default())
}

pub fn save(path: &Path, accounts: &[Account]) -> io::Result<()> {
    write_file(path, &format_accounts(accounts))
}

pub fn load_presets(path: &Path) -> io::Result<Vec<Preset>> {
    Ok(read_or_default(path)?
        .map(|t| parse_presets(&t))
        .unwrap_or_default())
}

pub fn save_presets(path: &Path, presets: &[Preset]) -> io::Result<()> {
    write_file(path, &format_presets(presets))
}

pub fn load_settings(path: &Path) -> io::Result<Settings> {
    Ok(read_or_default(path)?
        .map(|t| parse_settings(&t))
        .unwrap_or_default())
}

pub fn save_settings(path: &Path, settings: &Settings) -> io::Result<()> {
    write_file(path, &format_settings(settings))
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
            user_id: String::new(),
        }
    }

    #[test]
    fn round_trips_accounts() {
        let accounts = vec![
            account("alt1", "farming bank", "QmxvYg=="),
            account("alt2", "", ""),
        ];
        assert_eq!(parse_accounts(&format_accounts(&accounts)), accounts);
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

    #[test]
    fn round_trips_presets() {
        let presets = vec![
            Preset {
                name: "Bloxburg".into(),
                place_id: "185655149".into(),
            },
            Preset {
                name: "Farm".into(),
                place_id: "123".into(),
            },
        ];
        assert_eq!(parse_presets(&format_presets(&presets)), presets);
    }

    #[test]
    fn drops_presets_without_a_place_id() {
        assert!(parse_presets("NoPlace\n\tonly-id\n").is_empty());
    }

    #[test]
    fn keeps_user_id_and_rejects_bad_avatar_ids() {
        let mut acc = account("alt", "", "BLOB");
        acc.user_id = "12345".into();
        assert_eq!(parse_accounts(&format_accounts(&[acc.clone()])), vec![acc]);
        assert!(avatar_path("12345").is_some());
        assert!(avatar_path("../etc").is_none());
        assert!(avatar_path("").is_none());
    }

    #[test]
    fn round_trips_settings() {
        let settings = Settings {
            multi_instance: true,
            fps_cap: 144,
            launch_delay_ms: 750,
            graphics_level: 7,
            cpu_share_on_launch: true,
            cpu_priority_normal: true,
            ram_limit_mb: 512,
            cpu_limit_percent: 40,
            dark_mode: false,
        };
        assert_eq!(parse_settings(&format_settings(&settings)), settings);
    }

    #[test]
    fn missing_settings_use_defaults() {
        assert_eq!(parse_settings(""), Settings::default());
        assert_eq!(parse_settings("fps_cap=abc\nother=1"), Settings::default());
        assert_eq!(Settings::default().launch_delay_ms, 2000);
    }
}
