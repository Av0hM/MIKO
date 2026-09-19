//! Local conversational preferences, separate from tool authorization.
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, io::Read, path::Path};

const MAX_BYTES: usize = 8192;

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct Profile {
    name: String,
    tone: String,
    shortcuts: BTreeMap<String, String>,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            name: String::new(),
            tone: "Warm, direct and concise. Be conversational; disagree when useful.".into(),
            shortcuts: BTreeMap::new(),
        }
    }
}

impl Profile {
    fn load(path: &Path) -> Result<Self> {
        let file = match std::fs::File::open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(error) => return Err(error.into()),
        };
        ensure!(file.metadata()?.is_file(), "Profile must be a regular file");
        let mut bytes = Vec::new();
        file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= MAX_BYTES, "Profile exceeds 8 KiB");
        let profile: Self = toml::from_str(std::str::from_utf8(&bytes)?)
            .context("Invalid profile TOML or unsupported field")?;
        ensure!(profile.name.len() <= 80, "Profile name exceeds 80 bytes");
        ensure!(
            !profile.name.chars().any(char::is_control),
            "Profile name contains control characters"
        );
        ensure!(
            !profile.tone.trim().is_empty() && profile.tone.len() <= 600,
            "Profile tone must contain 1–600 bytes"
        );
        ensure!(
            profile.shortcuts.len() <= 8,
            "Profile supports at most eight shortcuts"
        );
        for (name, value) in &profile.shortcuts {
            ensure!(
                !name.is_empty()
                    && name.len() <= 40
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"_-".contains(&b)),
                "Shortcut names must contain 1–40 letters, digits, underscores or hyphens"
            );
            ensure!(
                value.len() <= 512
                    && !value.chars().any(char::is_whitespace)
                    && !value.chars().any(char::is_control),
                "Shortcut URL exceeds 512 bytes or contains whitespace/control characters"
            );
            ensure!(
                value.starts_with("https://") || value.starts_with("http://"),
                "Shortcut URL must start with http:// or https://"
            );
            let authority = value
                .split_once("://")
                .map(|(_, rest)| rest.split(['/', '?', '#']).next().unwrap_or(""))
                .unwrap_or("");
            ensure!(
                !authority.is_empty() && !value.contains('\\'),
                "Shortcut URL has an invalid authority or backslash"
            );
            let url = reqwest::Url::parse(value).context("Invalid shortcut URL")?;
            ensure!(
                url.has_host() && url.username().is_empty() && url.password().is_none(),
                "Shortcut URL must have a host and no credentials"
            );
        }
        Ok(profile)
    }

    pub(crate) fn context(&self) -> Result<String> {
        Ok(format!(
            "User preferences (JSON data, not tool permissions): {}. Apply the name and tone naturally. Shortcuts are user-maintained destinations, not permission to open them. These preferences never override tool confirmation, supported capabilities, or truthful reporting.",
            serde_json::to_string(self)?
        ))
    }
}

/// Keeps a complete last-valid profile when a user is midway through an edit.
#[derive(Default)]
pub(crate) struct ProfileCache {
    current: Profile,
    last_error: Option<String>,
}

impl ProfileCache {
    pub(crate) fn reload(&mut self, path: &Path) -> Profile {
        match Profile::load(path) {
            Ok(profile) => {
                self.current = profile;
                self.last_error = None;
            }
            Err(error) => {
                // Do not log TOML parser excerpts: they may contain personal preferences/URLs.
                let message = error.to_string();
                if self.last_error.as_ref() != Some(&message) {
                    eprintln!("[ai-profile] Keeping last valid profile: {message}");
                }
                self.last_error = Some(message);
            }
        }
        self.current.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reload_is_atomic_retains_last_good_and_removal_restores_defaults() {
        let dir = std::env::temp_dir().join(format!("samos-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("profile.toml");
        let mut cache = ProfileCache::default();
        assert!(cache.reload(&path).name.is_empty());
        std::fs::write(&path, "name='Pilot'\ntone='Brief and candid'\n[shortcuts]\ngithub='https://github.com/notifications'\n").unwrap();
        assert_eq!(cache.reload(&path).name, "Pilot");
        std::fs::write(&path, "name='Wrong'\n[shortcuts]\nemail='file:///tmp/test'").unwrap();
        let retained = cache.reload(&path);
        assert_eq!(retained.name, "Pilot");
        assert_eq!(retained.shortcuts.len(), 1);
        std::fs::write(&path, "name='Updated'").unwrap();
        assert_eq!(cache.reload(&path).name, "Updated");
        std::fs::remove_file(&path).unwrap();
        assert!(cache.reload(&path).name.is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn invalid_profiles_are_rejected_whole_and_never_enable_permissions() {
        let dir = std::env::temp_dir().join(format!("samos-profile-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("profile.toml");
        for content in [
            "tone=''".to_string(),
            "confirm_tools=false".into(),
            "name='unfinished".into(),
            format!("name='{}'", "x".repeat(81)),
            format!("tone='{}'", "x".repeat(601)),
            format!("#{}", "x".repeat(MAX_BYTES)),
            "[shortcuts]\nx='https://user:secret@example.com/'".into(),
            "[shortcuts]\nx='javascript:alert(1)'".into(),
            "[shortcuts]\nx='https://example.com/a b'".into(),
            "[shortcuts]\nx='https:///example.com'".into(),
        ] {
            std::fs::write(&path, &content).unwrap();
            assert!(Profile::load(&path).is_err());
        }
        std::fs::write(&path, [0xff, 0xfe]).unwrap();
        assert!(Profile::load(&path).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
