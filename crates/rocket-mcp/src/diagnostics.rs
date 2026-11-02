use serde_json::{Value, json};
use std::{
    fs::File,
    io::{BufRead, BufReader, ErrorKind, Read},
    path::Path,
};

// Match the logger's fixed retention paths: five 1 MiB rotation files plus
// up to 256 overflow records of 16 KiB. Never enumerate unrelated files.
pub fn read(directory: &Path, session: &str, after: u64, limit: usize) -> Result<Value, String> {
    let mut records = vec![];
    let mut oldest: Option<u64> = None;
    for path in rocket_git::DiagnosticLog::retained_files(directory)
        .map_err(|_| "Cannot read diagnostic retention paths")? {
        let maximum = if path.parent() == Some(directory) { 1024 * 1024 } else { 16_384 };
        let metadata = match path.symlink_metadata() {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == ErrorKind::NotFound => continue,
            Err(_) => {
                return Err(
                    "Diagnostics unavailable; read again after the operation settles".into(),
                );
            }
        };
        if !metadata.is_file() || metadata.len() > maximum {
            return Err("Diagnostic file is redirected or exceeds its bound".into());
        }
        let file =
            File::open(path).map_err(|_| "Cannot read diagnostics; the log may be rotating")?;
        let mut reader = BufReader::new(file.take(maximum));
        loop {
            let mut line = Vec::new();
            let count = (&mut reader)
                .take(16_385)
                .read_until(b'\n', &mut line)
                .map_err(|_| "Cannot read diagnostics")?;
            if count == 0 {
                break;
            }
            if count > 16_384 {
                return Err("Diagnostic record exceeds its bound".into());
            }
            // Only a final, incomplete line can be an in-progress logger write.
            if !line.ends_with(b"\n") {
                break;
            }
            let record: Value = serde_json::from_slice(&line)
                .map_err(|_| "Malformed complete diagnostic record")?;
            if record["session"] == session {
                let seq = record["seq"]
                    .as_u64()
                    .ok_or("Invalid diagnostic sequence")?;
                oldest = Some(oldest.map_or(seq, |previous| previous.min(seq)));
                if seq > after {
                    records.push(record);
                }
            }
        }
    }
    records.sort_by_key(|r| r["seq"].as_u64());
    records.dedup_by_key(|r| r["seq"].as_u64());
    let more = records.len() > limit;
    records.truncate(limit);
    Ok(json!({"session":session,
        "oldest_seq":oldest,"retention_gap":oldest.is_some_and(|seq| seq > after.saturating_add(1)),
        "next_seq":records.last().and_then(|r| r["seq"].as_u64()).unwrap_or(after),
        "records":records,"more":more}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn durable_overflow_is_visible_before_recovery_and_deduplicated_afterwards() {
        let root = std::env::temp_dir().join(format!("rocket-overflow-read-{}-{}", std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let log = rocket_git::DiagnosticLog::open(&root).unwrap();
        let lock = std::fs::OpenOptions::new().read(true).write(true).open(root.join("rocket.lock")).unwrap();
        lock.try_lock().unwrap();
        log.request("status", |_| Ok(())).unwrap();
        let before = read(&root, log.session_id(), 0, 20).unwrap();
        assert_eq!(before["records"].as_array().unwrap().len(), 3);
        assert_eq!(log.health().dropped_events, 0);
        drop(lock);
        log.request("status", |_| Ok(())).unwrap();
        let after = read(&root, log.session_id(), 0, 20).unwrap();
        assert_eq!(after["records"].as_array().unwrap().len(), 5);
        for file in rocket_git::DiagnosticLog::retained_files(&root).unwrap() {
            if file.exists() { std::fs::remove_file(file).unwrap(); }
        }
        std::fs::remove_file(root.join("rocket.lock")).unwrap();
        std::fs::remove_dir(root.join("rocket-overflow")).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
    #[test]
    fn logs_are_scoped_paginated_and_do_not_hide_corruption_or_retention() {
        let root = std::env::temp_dir().join(format!(
            "rocket-mcp-logs-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&root).unwrap();
        let current = root.join("rocket.jsonl");
        let older = root.join("rocket.1.jsonl");
        let unrelated = root.join("unrelated.jsonl");
        std::fs::write(&unrelated, "private unrelated file\n").unwrap();
        std::fs::write(&older, "{\"session\":\"active\",\"seq\":8}\n").unwrap();
        std::fs::write(
            &current,
            "{\"session\":\"active\",\"seq\":9}\n{\"session\":\"other\",\"seq\":1}\n{\"partial\":",
        )
        .unwrap();
        let first = read(&root, "active", 0, 1).unwrap();
        assert_eq!(first["retention_gap"], true);
        assert_eq!(first["next_seq"], 8);
        assert_eq!(first["more"], true);
        let second = read(&root, "active", 8, 1).unwrap();
        assert_eq!(second["records"].as_array().unwrap().len(), 1);
        assert_eq!(second["next_seq"], 9);
        assert_eq!(second["retention_gap"], false);
        std::fs::write(&current, "malformed complete line\n").unwrap();
        assert!(
            read(&root, "active", 0, 1)
                .unwrap_err()
                .contains("Malformed")
        );
        std::fs::write(&current, vec![b'x'; 16_385]).unwrap();
        assert!(
            read(&root, "active", 0, 1)
                .unwrap_err()
                .contains("record exceeds")
        );
        std::fs::write(&current, vec![b'x'; 1024 * 1024 + 1]).unwrap();
        assert!(
            read(&root, "active", 0, 1)
                .unwrap_err()
                .contains("file is redirected or exceeds")
        );
        for path in [current, older, unrelated] {
            std::fs::remove_file(path).unwrap();
        }
        std::fs::remove_dir(root).unwrap();
    }
}
