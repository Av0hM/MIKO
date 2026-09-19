//! Small local UTC iCalendar vdir. Unsupported imported recurrence/timezones fail explicitly.
use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::Mutex,
};
static WRITE_LOCK: Mutex<()> = Mutex::new(());
#[derive(Clone, Serialize, Deserialize, Debug)]
pub(crate) struct Event {
    pub id: String,
    pub title: String,
    pub start: i64,
    pub end: i64,
}
fn directory(home: &Path) -> PathBuf {
    home.join(".local/share/calendars/personal")
}
fn timestamp(value: &str) -> Result<i64> {
    Ok(chrono::DateTime::parse_from_rfc3339(value)
        .context("Time must be RFC3339 with explicit UTC offset")?
        .timestamp())
}
fn unescape(value: &str) -> Result<String> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('n' | 'N') => out.push('\n'),
                Some(c @ ('\\' | ',' | ';')) => out.push(c),
                _ => bail!("Invalid calendar text escape"),
            }
        } else {
            out.push(c)
        }
    }
    Ok(out)
}
fn escape(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace(';', "\\;")
        .replace(',', "\\,")
}
fn utc(value: &str) -> Result<i64> {
    Ok(
        chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")?
            .and_utc()
            .timestamp(),
    )
}
fn parse(text: &str) -> Result<Event> {
    let mut lines: Vec<String> = vec![];
    for line in text.lines() {
        if line.starts_with([' ', '\t']) {
            lines
                .last_mut()
                .context("Invalid folded line")?
                .push_str(&line[1..]);
        } else {
            lines.push(line.into());
        }
    }
    ensure!(
        lines
            .iter()
            .filter(|s| s.as_str() == "BEGIN:VEVENT")
            .count()
            == 1,
        "Calendar file must contain exactly one event"
    );
    let mut fields = std::collections::HashMap::new();
    let mut inside = false;
    for line in lines {
        if line == "BEGIN:VEVENT" {
            inside = true;
            continue;
        }
        if line == "END:VEVENT" {
            inside = false;
            continue;
        }
        if !inside {
            continue;
        }
        let (key, value) = line.split_once(':').context("Malformed event line")?;
        ensure!(
            !key.starts_with("RRULE")
                && !key.starts_with("RDATE")
                && !key.starts_with("EXDATE")
                && !key.starts_with("RECURRENCE-ID"),
            "Recurring imported events need a recurrence-aware calendar backend; scheduling is blocked to avoid missed conflicts"
        );
        if key.starts_with("DTSTART") || key.starts_with("DTEND") {
            ensure!(
                key == "DTSTART" || key == "DTEND",
                "Imported local-time/all-day events are not supported yet; scheduling blocked"
            );
        }
        ensure!(
            fields.insert(key.to_string(), value.to_string()).is_none(),
            "Duplicate event field"
        );
    }
    let get = |key: &str| {
        fields
            .get(key)
            .map(String::as_str)
            .with_context(|| format!("Missing event {key}"))
    };
    let e = Event {
        id: get("UID")?.into(),
        title: unescape(get("SUMMARY")?)?,
        start: utc(get("DTSTART")?)?,
        end: utc(get("DTEND")?)?,
    };
    ensure!(e.end > e.start, "Invalid event interval");
    Ok(e)
}
pub(crate) fn events(home: &Path) -> Result<Vec<Event>> {
    let dir = directory(home);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut events = vec![];
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.path().extension().and_then(|s| s.to_str()) != Some("ics") {
            continue;
        }
        ensure!(
            entry.file_type()?.is_file(),
            "Calendar entries must be regular files"
        );
        ensure!(entry.metadata()?.len() <= 65536, "Calendar file too large");
        events.push(
            parse(&std::fs::read_to_string(entry.path())?)
                .with_context(|| format!("Cannot safely read {}", entry.path().display()))?,
        );
        ensure!(events.len() <= 500, "Calendar limit is 500 events");
    }
    events.sort_by(|a, b| (a.start, &a.id).cmp(&(b.start, &b.id)));
    Ok(events)
}
pub(crate) fn list(home: &Path, args: &Value) -> Result<Value> {
    let start = match args["start"].as_str() {
        Some(s) => timestamp(s)?,
        None => chrono::Utc::now().timestamp(),
    };
    let end = match args["end"].as_str() {
        Some(s) => timestamp(s)?,
        None => start + 7 * 86400,
    };
    ensure!(end > start, "End must be after start");
    Ok(
        json!({"events":events(home)?.into_iter().filter(|e|e.start<end&&e.end>start).collect::<Vec<_>>(),"time_units":"UTC Unix seconds; local timezone display must be explicit"}),
    )
}
pub(crate) fn preview(home: &Path, name: &str, args: &mut Value) -> Result<()> {
    let all = events(home)?;
    if name == "schedule_meeting" {
        let title = args["title"].as_str().context("title required")?;
        ensure!(
            !title.trim().is_empty() && title.len() <= 500 && !title.contains('\r'),
            "Invalid title"
        );
        let start = timestamp(args["start"].as_str().context("start required")?)?;
        let end = timestamp(args["end"].as_str().context("end required")?)?;
        ensure!(
            end > start && end - start <= 31 * 86400,
            "Event duration must be positive and at most 31 days"
        );
        args["conflicts"] = json!(
            all.iter()
                .filter(|e| e.start < end && e.end > start)
                .collect::<Vec<_>>()
        );
    } else {
        let id = args["id"].as_str().context("Event id required")?;
        args["event"] = json!(all.iter().find(|e| e.id == id).context("Event not found")?);
    }
    args["_snapshot"] = json!(all);
    Ok(())
}
pub(crate) fn apply(home: &Path, name: &str, args: &Value) -> Result<String> {
    let _lock = WRITE_LOCK
        .lock()
        .map_err(|_| anyhow::anyhow!("Calendar unavailable"))?;
    ensure!(
        args.get("_snapshot").is_some(),
        "Calendar changes need a preview"
    );
    ensure!(
        json!(events(home)?) == args["_snapshot"],
        "Calendar changed since preview; request a new confirmation"
    );
    let dir = directory(home);
    std::fs::create_dir_all(&dir)?;
    if name == "cancel_meeting" {
        let id = args["id"].as_str().context("Event id required")?;
        // Remove only our own UID-based file; imported events are never deleted by guessed paths.
        let uuid = uuid::Uuid::parse_str(id).context("Only SamOS UUID events can be cancelled")?;
        let path = dir.join(format!("{uuid}.ics"));
        let event = parse(&std::fs::read_to_string(&path)?)?;
        ensure!(event.id == id, "Event ID mismatch");
        std::fs::remove_file(path)?;
        return Ok(format!("Cancelled {}", event.title));
    }
    let mut reviewed = args.clone();
    preview(home, name, &mut reviewed)?;
    let event = Event {
        id: uuid::Uuid::new_v4().to_string(),
        title: args["title"].as_str().unwrap().into(),
        start: timestamp(args["start"].as_str().unwrap())?,
        end: timestamp(args["end"].as_str().unwrap())?,
    };
    let dt = |t| {
        chrono::DateTime::from_timestamp(t, 0)
            .map(|d| d.format("%Y%m%dT%H%M%SZ").to_string())
            .context("Invalid date")
    };
    let content = format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//SamOS//MIKO//EN\r\nBEGIN:VEVENT\r\nUID:{}\r\nDTSTAMP:{}\r\nDTSTART:{}\r\nDTEND:{}\r\nSUMMARY:{}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n",
        event.id,
        dt(chrono::Utc::now().timestamp())?,
        dt(event.start)?,
        dt(event.end)?,
        escape(&event.title)
    );
    let temp = dir.join(format!(".{}.tmp", event.id));
    let dest = dir.join(format!("{}.ics", event.id));
    let result = (|| -> Result<()> {
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temp)?;
        // Fold UTF-8 safely within the iCalendar 75-octet line limit.
        for line in content.split("\r\n").filter(|s| !s.is_empty()) {
            let mut bytes = 0;
            for c in line.chars() {
                if bytes + c.len_utf8() > 74 {
                    f.write_all(b"\r\n ")?;
                    bytes = 1;
                }
                write!(f, "{c}")?;
                bytes += c.len_utf8();
            }
            f.write_all(b"\r\n")?;
        }
        f.sync_all()?;
        std::fs::rename(&temp, &dest)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temp);
    }
    result?;
    Ok(serde_json::to_string(&event)?)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn schedule_conflicts_stale_preview_and_cancel() {
        let home = std::env::temp_dir().join(format!("samos-calendar-{}", uuid::Uuid::new_v4()));
        let mut a = json!({"title":"Tea, notes; café\nplanning","start":"2026-10-01T10:00:00+05:30","end":"2026-10-01T11:00:00+05:30"});
        preview(&home, "schedule_meeting", &mut a).unwrap();
        assert!(events(&home).unwrap().is_empty());
        let saved: Event =
            serde_json::from_str(&apply(&home, "schedule_meeting", &a).unwrap()).unwrap();
        assert_eq!(events(&home).unwrap()[0].title, saved.title);
        assert!(apply(&home, "schedule_meeting", &a).is_err());
        let mut b = a.clone();
        preview(&home, "schedule_meeting", &mut b).unwrap();
        assert_eq!(b["conflicts"].as_array().unwrap().len(), 1);
        b["start"] = json!("2026-10-01T11:00:00+05:30");
        b["end"] = json!("2026-10-01T12:00:00+05:30");
        preview(&home, "schedule_meeting", &mut b).unwrap();
        assert!(b["conflicts"].as_array().unwrap().is_empty());
        let mut cancel = json!({"id":saved.id});
        preview(&home, "cancel_meeting", &mut cancel).unwrap();
        apply(&home, "cancel_meeting", &cancel).unwrap();
        assert!(events(&home).unwrap().is_empty());
        std::fs::remove_dir_all(home).unwrap();
    }
}
