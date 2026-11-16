//! Calls the C# helper (RobloxProcessLookup.exe). Cookies travel on stdin, never on the command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

/// The helper is expected next to this program.
pub fn default_helper_path() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join("RobloxProcessLookup.exe")))
        .unwrap_or_else(|| PathBuf::from("RobloxProcessLookup.exe"))
}

fn run(helper: &Path, args: &[&str], input_line: &str) -> Result<String, String> {
    let mut child = Command::new(helper)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", helper.display()))?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(input_line.as_bytes())
            .map_err(|e| format!("could not send input to helper: {e}"))?;
        stdin
            .write_all(b"\n")
            .map_err(|e| format!("could not send input to helper: {e}"))?;
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("helper did not finish: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if stderr.is_empty() {
            Err(format!("helper exited with {}", output.status))
        } else {
            Err(stderr)
        }
    }
}

/// What the helper returns for a cookie: the encrypted blob, plus the account's name and user ID.
#[derive(Debug, Clone, PartialEq)]
pub struct AccountInfo {
    pub blob: String,
    pub username: String,
    pub user_id: String,
}

/// The helper prints the blob, username, and user ID on separate lines (the last two may be empty).
fn parse_account_info(output: &str) -> Result<AccountInfo, String> {
    let mut lines = output.lines().map(|l| l.trim().to_string());
    let blob = lines.next().unwrap_or_default();
    if blob.is_empty() {
        return Err("helper returned no cookie data".into());
    }
    Ok(AccountInfo {
        blob,
        username: lines.next().unwrap_or_default(),
        user_id: lines.next().unwrap_or_default(),
    })
}

/// Encrypts a pasted cookie and looks up the account.
pub fn protect(helper: &Path, cookie: &str) -> Result<AccountInfo, String> {
    parse_account_info(&run(helper, &["protect"], cookie.trim())?)
}

/// Opens a Roblox login window and returns the account once the user has signed in.
pub fn login(helper: &Path) -> Result<AccountInfo, String> {
    parse_account_info(&run(helper, &["login"], "")?)
}

/// Saves the account's headshot to `destination` (a PNG file).
pub fn avatar(helper: &Path, user_id: &str, destination: &Path) -> Result<(), String> {
    if let Some(dir) = destination.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let dest = destination.to_string_lossy().into_owned();
    run(helper, &["avatar", user_id, &dest], "").map(|_| ())
}

/// Sets the Roblox FPS cap. 0 removes it.
pub fn set_fps(helper: &Path, fps: u32) -> Result<(), String> {
    run(helper, &["fps", &fps.to_string()], "").map(|_| ())
}

/// Sets the graphics quality override. 0 removes it.
pub fn set_graphics(helper: &Path, level: u32) -> Result<(), String> {
    run(helper, &["graphics", &level.to_string()], "").map(|_| ())
}

/// Writes the user's client flags (a JSON object). Returns an error if the JSON isn't valid.
pub fn apply_flags(helper: &Path, json: &str) -> Result<(), String> {
    run(helper, &["flags"], json).map(|_| ())
}

/// Spreads running Roblox windows across CPU cores. Returns how many were set.
pub fn spread_across_cores(helper: &Path) -> Result<String, String> {
    run(helper, &["spread"], "")
}

/// Resizes every Roblox window. Returns the size Windows applied to each, one per line.
pub fn resize_windows(helper: &Path, width: u32, height: u32) -> Result<String, String> {
    run(
        helper,
        &["resize", &width.to_string(), &height.to_string()],
        "",
    )
}

/// Lowers the CPU priority of running Roblox windows. Returns how many were changed.
pub fn lower_priority(helper: &Path) -> Result<String, String> {
    run(helper, &["priority"], "")
}

/// Starts the helper that holds Roblox's single-instance lock. The caller keeps the child and kills it to turn this off.
pub fn start_multi_instance(helper: &Path) -> Result<Child, String> {
    Command::new(helper)
        .arg("multi-instance")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start multi-instance lock: {e}"))
}

/// Starts the game for the account whose cookie blob is given.
/// An empty `job_id` joins any server; a job ID joins that specific server.
pub fn launch(helper: &Path, blob: &str, place_id: u64, job_id: &str) -> Result<(), String> {
    let place = place_id.to_string();
    let job = job_id.trim();
    let args: Vec<&str> = if job.is_empty() {
        vec!["launch", place.as_str()]
    } else {
        vec!["launch", place.as_str(), job]
    };
    run(helper, &args, blob).map(|_| ())
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake helper that echoes what it received, so the plumbing can be tested on Linux.
    fn fake_helper(test: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rbx-helper-test-{}-{}", std::process::id(), test));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("fake-helper.sh");
        std::fs::write(
            &path,
            "#!/bin/sh\nread line\nif [ \"$1\" = protect ]; then printf 'BLOB:%s\\nalt_name\\n' \"$line\"; else echo \"LAUNCHED:$2:$line\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn protect_returns_blob_and_username() {
        let helper = fake_helper("protect");
        let info = protect(&helper, "COOKIE").unwrap();
        assert_eq!(info.blob, "BLOB:COOKIE");
        assert_eq!(info.username, "alt_name");
        assert_eq!(info.user_id, "");
    }

    #[test]
    fn launch_sends_place_and_blob() {
        let helper = fake_helper("launch");
        assert!(launch(&helper, "BLOB", 42, "").is_ok());
        assert!(launch(&helper, "BLOB", 42, "job-123").is_ok());
    }

    #[test]
    fn missing_helper_is_an_error_not_a_panic() {
        assert!(protect(Path::new("/nonexistent/helper"), "x").is_err());
    }
}
