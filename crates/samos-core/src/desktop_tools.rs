//! Bounded read-only desktop queries and validated action arguments.
use anyhow::{Result, ensure};
use serde_json::{Value, json};

pub(crate) fn system_query(sort: &str, limit: usize) -> Result<Value> {
    ensure!(
        ["memory", "cpu"].contains(&sort),
        "sort must be memory or cpu"
    );
    ensure!((1..=20).contains(&limit), "limit must be 1–20");
    let mut system = sysinfo::System::new();
    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    if sort == "cpu" {
        std::thread::sleep(sysinfo::MINIMUM_CPU_UPDATE_INTERVAL);
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
    }
    let mut rows:Vec<_>=system.processes().iter().filter(|(_,p)|p.thread_kind().is_none()).map(|(pid,p)|json!({"pid":pid.as_u32(),"name":p.name().to_string_lossy(),"memory_bytes":p.memory(),"cpu_percent":p.cpu_usage()})).collect();
    let key = if sort == "cpu" {
        "cpu_percent"
    } else {
        "memory_bytes"
    };
    rows.sort_by(|a, b| {
        b[key]
            .as_f64()
            .unwrap_or_default()
            .total_cmp(&a[key].as_f64().unwrap_or_default())
    });
    rows.truncate(limit);
    Ok(
        json!({"sort":sort,"processes":rows,"cpu_units":"percent of one logical CPU; can exceed 100 on multicore","memory_units":"resident bytes","sampled_at":chrono::Utc::now().to_rfc3339()}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn process_query_is_bounded_and_omits_command_lines() {
        assert!(system_query("invalid", 5).is_err());
        assert!(system_query("memory", 21).is_err());
        let result = system_query("memory", 3).unwrap();
        let rows = result["processes"].as_array().unwrap();
        assert!(rows.len() <= 3);
        assert!(rows.iter().all(|r| r.get("command").is_none()));
        assert!(
            rows.windows(2)
                .all(|r| r[0]["memory_bytes"].as_u64() >= r[1]["memory_bytes"].as_u64())
        );
    }
}

pub(crate) fn find_files(
    home: &std::path::Path,
    root: &str,
    query: &str,
    days: Option<u64>,
) -> Result<Value> {
    use std::time::{Duration, Instant, SystemTime};
    ensure!(
        !query.trim().is_empty() && query.len() <= 200,
        "query must contain 1–200 bytes"
    );
    ensure!(
        days.is_none_or(|v| (1..=3650).contains(&v)),
        "modified_within_days must be 1–3650"
    );
    let home = home.canonicalize()?;
    let root = if root.is_empty() {
        home.clone()
    } else {
        std::path::PathBuf::from(root).canonicalize()?
    };
    ensure!(
        root.starts_with(&home) && root.is_dir(),
        "Search root must be a directory inside your home"
    );
    let mut pending = vec![root];
    let mut files = vec![];
    let mut scanned = 0usize;
    let start = Instant::now();
    let needle = query.to_lowercase();
    let mut truncated = false;
    'walk: while let Some(dir) = pending.pop() {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            scanned += 1;
            if scanned > 20000 || start.elapsed() > Duration::from_secs(2) || files.len() >= 50 {
                truncated = true;
                break 'walk;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.')
                || ["node_modules", "target", "__pycache__"].contains(&name.as_str())
            {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                pending.push(entry.path());
                continue;
            }
            if !kind.is_file() || !name.to_lowercase().contains(&needle) {
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if let Some(days) = days {
                if !meta
                    .modified()
                    .ok()
                    .and_then(|m| SystemTime::now().duration_since(m).ok())
                    .is_some_and(|a| a.as_secs() <= days * 86400)
                {
                    continue;
                }
            }
            files.push(json!({"path":entry.path(),"size_bytes":meta.len()}));
        }
    }
    Ok(
        json!({"files":files,"truncated":truncated,"scanned_entries":scanned,"search":"filename substring; optional modification time, not last-read time; hidden files and symlinks excluded"}),
    )
}

pub(crate) fn openable_file(
    home: &std::path::Path,
    path: &str,
) -> Result<(std::path::PathBuf, String)> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let path = std::path::Path::new(path).canonicalize()?;
    ensure!(
        path.starts_with(home.canonicalize()?),
        "File must be inside your home"
    );
    let meta = path.metadata()?;
    ensure!(
        meta.is_file() && meta.permissions().mode() & 0o111 == 0,
        "Only non-executable regular documents can be opened"
    );
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    ensure!(
        [
            "pdf", "txt", "md", "png", "jpg", "jpeg", "svg", "webp", "csv", "json", "toml", "yaml",
            "yml", "docx", "xlsx", "pptx", "odt", "ods", "mp3", "mp4", "wav", "flac"
        ]
        .contains(&extension.as_str()),
        "Unsupported document type"
    );
    let identity = format!(
        "{}:{}:{}:{}:{}",
        meta.dev(),
        meta.ino(),
        meta.len(),
        meta.mtime(),
        meta.mtime_nsec()
    );
    Ok((path, identity))
}

#[cfg(test)]
mod file_tests {
    use super::*;
    #[test]
    fn search_is_bounded_and_open_rejects_escape_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("samos-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("notes.pdf"), "fixture").unwrap();
        std::fs::write(dir.join(".secret.pdf"), "hidden").unwrap();
        let results = find_files(&dir, "", "pdf", None).unwrap();
        assert_eq!(results["files"].as_array().unwrap().len(), 1);
        assert!(openable_file(&dir, dir.join("notes.pdf").to_str().unwrap()).is_ok());
        assert!(openable_file(&dir, "/etc/passwd").is_err());
        std::fs::set_permissions(
            dir.join("notes.pdf"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(openable_file(&dir, dir.join("notes.pdf").to_str().unwrap()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}

pub(crate) fn browser_url(browser: &str, url: &str) -> Result<(String, String)> {
    let binary = match browser {
        "firefox" => "firefox",
        "vivaldi" => "vivaldi-stable",
        _ => anyhow::bail!("Choose firefox or vivaldi explicitly"),
    };
    ensure!(
        url.len() <= 2048
            && !url.chars().any(|c| c.is_whitespace() || c.is_control())
            && !url.contains('\\'),
        "Invalid URL"
    );
    ensure!(
        url.starts_with("https://") || url.starts_with("http://"),
        "Only HTTP(S) browser URLs are allowed"
    );
    let authority = url
        .split_once("://")
        .unwrap()
        .1
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("");
    ensure!(!authority.is_empty(), "URL must have a host");
    let parsed = reqwest::Url::parse(url)?;
    ensure!(
        parsed.has_host() && parsed.username().is_empty() && parsed.password().is_none(),
        "URL must have a host and no embedded credentials"
    );
    Ok((binary.into(), parsed.into()))
}
#[cfg(test)]
mod browser_tests {
    use super::*;
    #[test]
    fn validates_browser_and_url_before_handoff() {
        assert!(browser_url("firefox", "https://example.com").is_ok());
        for (browser, url) in [
            ("default", "https://example.com"),
            ("firefox", "file:///tmp/run"),
            ("vivaldi", "https://user:pw@example.com"),
            ("firefox", "https:///example.com"),
            ("firefox", "javascript:alert(1)"),
        ] {
            assert!(browser_url(browser, url).is_err());
        }
    }
}
