//! Calls the C# helper (RobloxProcessLookup.exe) for cookie encryption and launching.
//! Cookies travel to the helper on stdin, never on the command line.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

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

/// Encrypts a cookie. Returns the blob to store.
pub fn protect(helper: &Path, cookie: &str) -> Result<String, String> {
    run(helper, &["protect"], cookie.trim())
}

/// Starts the game for the account whose cookie blob is given.
pub fn launch(helper: &Path, blob: &str, place_id: u64) -> Result<(), String> {
    run(helper, &["launch", &place_id.to_string()], blob).map(|_| ())
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
            "#!/bin/sh\nread line\nif [ \"$1\" = protect ]; then echo \"BLOB:$line\"; else echo \"LAUNCHED:$2:$line\"; fi\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn protect_sends_cookie_and_returns_blob() {
        let helper = fake_helper("protect");
        assert_eq!(protect(&helper, "COOKIE").unwrap(), "BLOB:COOKIE");
    }

    #[test]
    fn launch_sends_place_and_blob() {
        let helper = fake_helper("launch");
        assert!(launch(&helper, "BLOB", 42).is_ok());
    }

    #[test]
    fn missing_helper_is_an_error_not_a_panic() {
        let result = protect(Path::new("/nonexistent/helper"), "x");
        assert!(result.is_err());
    }
}
