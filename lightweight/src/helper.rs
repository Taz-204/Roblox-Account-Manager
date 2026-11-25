//! Calls the C# helper (RobloxProcessLookup.exe). Cookies travel on stdin, never on the command line.

use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::Mutex;

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
    /// Why Roblox gave no username (for example "HTTP 401"). Empty when the lookup worked.
    pub reason: String,
}

/// The helper prints the blob, username, user ID and lookup reason on separate lines (all but the blob may be empty).
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
        reason: lines.next().unwrap_or_default(),
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
/// Starts a client and returns its process ID (0 if it's unknown, e.g. for the link fallback).
pub fn launch(helper: &Path, blob: &str, place_id: u64, job_id: &str) -> Result<u32, String> {
    let place = place_id.to_string();
    let job = job_id.trim();
    let args: Vec<&str> = if job.is_empty() {
        vec!["launch", place.as_str()]
    } else {
        vec!["launch", place.as_str(), job]
    };
    let output = run(helper, &args, blob)?;
    // The helper answers with {"ok":true,"pid":N}.
    let pid = output
        .split("\"pid\":")
        .nth(1)
        .unwrap_or("")
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    Ok(pid)
}

/// Reads the helper's reply: {"ok":true,"pid":N} or {"ok":false,"error":"..."}.
fn parse_launch_reply(reply: &str) -> Result<u32, String> {
    if reply.contains("\"ok\":false") {
        let rest = reply
            .split("\"error\":\"")
            .nth(1)
            .unwrap_or("launch failed");
        let end = rest.rfind("\"}").unwrap_or(rest.len());
        return Err(rest[..end].replace("\\\"", "\"").replace("\\\\", "\\"));
    }
    // The helper answers with {"ok":true,"pid":N}. 0 means the pid is unknown.
    let pid = reply
        .split("\"pid\":")
        .nth(1)
        .unwrap_or("")
        .chars()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .parse()
        .unwrap_or(0);
    Ok(pid)
}

/// A running `RobloxProcessLookup.exe serve`. It is kept between launches so .NET starts only once.
struct Server {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

static SERVER: Mutex<Option<Server>> = Mutex::new(None);

fn start_server(helper: &Path) -> Result<Server, String> {
    let mut child = Command::new(helper)
        .arg("serve")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", helper.display()))?;
    let stdin = child
        .stdin
        .take()
        .ok_or("could not open the helper's input")?;
    let stdout = child
        .stdout
        .take()
        .ok_or("could not open the helper's output")?;
    Ok(Server {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

impl Server {
    fn request(&mut self, line: &str) -> Result<String, String> {
        self.stdin
            .write_all(line.as_bytes())
            .and_then(|_| self.stdin.flush())
            .map_err(|e| format!("launcher stopped: {e}"))?;
        let mut reply = String::new();
        let read = self
            .stdout
            .read_line(&mut reply)
            .map_err(|e| format!("launcher stopped: {e}"))?;
        if read == 0 {
            return Err("launcher stopped".into());
        }
        Ok(reply)
    }
}

/// Same as `launch`, but reuses one helper process across launches, so .NET starts only once.
/// The app uses this; the command line uses `launch`.
pub fn launch_fast(helper: &Path, blob: &str, place_id: u64, job_id: &str) -> Result<u32, String> {
    let line = format!("{place_id}\t{}\t{blob}\n", job_id.trim());
    let mut slot = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    for attempt in 0..2 {
        if slot.is_none() {
            *slot = Some(start_server(helper)?);
        }
        let server = slot.as_mut().expect("the launcher was just started");
        match server.request(&line) {
            Ok(reply) => return parse_launch_reply(&reply),
            Err(e) => {
                // The helper went away. Start a fresh one and try once more.
                if let Some(mut dead) = slot.take() {
                    let _ = dead.child.kill();
                    let _ = dead.child.wait();
                }
                if attempt == 1 {
                    return Err(e);
                }
            }
        }
    }
    Err("could not reach the launcher".into())
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
    fn parses_launch_replies() {
        assert_eq!(
            parse_launch_reply("{\"ok\":true,\"pid\":4321}\n").unwrap(),
            4321
        );
        assert_eq!(parse_launch_reply("{\"ok\":true}\n").unwrap(), 0);
        let err = parse_launch_reply("{\"ok\":false,\"error\":\"bad \\\"x\\\"\"}\n");
        assert_eq!(err.unwrap_err(), "bad \"x\"");
    }

    #[test]
    fn missing_helper_is_an_error_not_a_panic() {
        assert!(protect(Path::new("/nonexistent/helper"), "x").is_err());
    }
}
