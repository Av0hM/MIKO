//! Bounded subprocess execution shared by desktop tools and metric collectors.
use anyhow::{Result, ensure};
use std::io::Write;
use std::process::{Command, Stdio};

/// Run a program with literal arguments, a deadline, and optional standard input.
pub fn run(program: &str, args: &[&str], input: Option<&str>, seconds: u64) -> Result<String> {
    let mut child = Command::new("timeout")
        .args(["--kill-after=1s", &format!("{seconds}s"), program])
        .args(args).env("LC_ALL", "C")
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    if let Some(text) = input {
        if let Some(mut stdin) = child.stdin.take() {
            // A dedicated writer allows the timeout process to terminate a stalled reader.
            let text = text.to_owned();
            std::thread::spawn(move || { let _ = stdin.write_all(text.as_bytes()); });
        }
    }
    let output = child.wait_with_output()?;
    ensure!(output.status.success(), "{program} failed ({}): {}", output.status, String::from_utf8_lossy(&output.stderr).trim());
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}
