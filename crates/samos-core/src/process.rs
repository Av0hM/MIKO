//! Bounded subprocess execution shared by desktop tools and metric collectors.
use anyhow::{Result, ensure};
use std::io::Write;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// Run a program with literal arguments, a deadline, and optional standard input.
pub fn run(program: &str, args: &[&str], input: Option<&str>, seconds: u64) -> Result<String> {
    run_cancellable(program, args, input, seconds, &AtomicBool::new(false))
}

/// Run a bounded child process group, terminating it when its owner's stop flag is set.
pub fn run_cancellable(
    program: &str,
    args: &[&str],
    input: Option<&str>,
    seconds: u64,
    stop: &AtomicBool,
) -> Result<String> {
    ensure!(!stop.load(Ordering::Acquire), "SamOS is shutting down");
    let mut child = Command::new("timeout")
        .process_group(0)
        .args(["--kill-after=1s", &format!("{seconds}s"), program])
        .args(args)
        .env("LC_ALL", "C")
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(text) = input {
        if let Some(mut stdin) = child.stdin.take() {
            // A dedicated writer allows the timeout process to terminate a stalled reader.
            let text = text.to_owned();
            std::thread::spawn(move || {
                let _ = stdin.write_all(text.as_bytes());
            });
        }
    }
    let pid = child.id();
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    fn drain(mut source: impl std::io::Read) -> std::io::Result<Vec<u8>> {
        let mut retained = Vec::new();
        let mut buffer = [0; 8192];
        loop {
            let n = source.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            let keep = n.min(262144usize.saturating_sub(retained.len()));
            retained.extend_from_slice(&buffer[..keep]);
        }
        Ok(retained)
    }
    let out = std::thread::spawn(move || drain(stdout));
    let err = std::thread::spawn(move || drain(stderr));
    let cancelled = loop {
        if stop.load(Ordering::Acquire) {
            let _ = Command::new("/usr/bin/kill")
                .args(["-KILL", "--", &format!("-{pid}")])
                .status();
            let _ = child.kill();
            break true;
        }
        if child.try_wait()?.is_some() {
            break false;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let status = child.wait()?;
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("stdout reader failed"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("stderr reader failed"))??;
    ensure!(!cancelled, "SamOS is shutting down; command cancelled");
    ensure!(
        status.success(),
        "{program} failed ({}): {}",
        status,
        String::from_utf8_lossy(&stderr).trim()
    );
    Ok(String::from_utf8_lossy(&stdout).trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_stops_a_long_running_child_promptly() {
        let stop = std::sync::Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let start = std::time::Instant::now();
        let worker =
            std::thread::spawn(move || run_cancellable("sleep", &["30"], None, 35, &worker_stop));
        std::thread::sleep(Duration::from_millis(100));
        stop.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        assert!(start.elapsed() < Duration::from_secs(3));
    }
}
