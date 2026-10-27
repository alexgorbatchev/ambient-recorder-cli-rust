use ambient_recorder::{
    journal::{Journal, Level, Record},
    storage::Store,
};
use chrono::{TimeZone, Utc};
use serde_json::json;
use std::{
    io::{self, Write},
    sync::{Arc, Mutex},
};
#[derive(Clone, Default)]
struct Writer(Arc<Mutex<Vec<u8>>>);
impl Write for Writer {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn record(level: Level, msg: &str) -> Record {
    Record::new(chrono::Local.with_ymd_and_hms(2026,10,14,12,0,0).unwrap().with_timezone(&Utc),level,msg,json!({"microphone_name":"USB\u{1b}[2J\nMicrophone","microphone_uid":"native-uid","error":"first\nsecond","playback":"system"})).unwrap()
}
#[test]
fn daily_diagnostics_are_complete_while_console_is_filtered_and_control_safe() {
    for agent in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(dir.path()).unwrap());
        let writer = Writer::default();
        let mut journal = Journal::new(store, Some(Box::new(writer.clone())), agent, false);
        for level in [Level::Debug, Level::Info, Level::Warn, Level::Error] {
            journal.event(record(level, "capture\nforged")).unwrap();
        }
        journal
            .event(record(Level::Info, "Microphone available"))
            .unwrap();
        journal.sync().unwrap();
        journal.close().unwrap();
        let console = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
        assert_eq!(console.lines().count(), if agent { 4 } else { 3 });
        if agent {
            for line in console.lines() {
                let event: serde_json::Value = serde_json::from_str(line).unwrap();
                assert_eq!(event["microphone_uid"], "native-uid");
                assert!(
                    event["msg"] == "capture\nforged" || event["msg"] == "Microphone available"
                );
            }
        } else {
            assert!(!console.contains('\u{1b}'));
            assert!(console.contains("\\nforged"));
            assert!(console.contains("microphone="));
            assert!(console.contains("all applications"));
            assert!(!console.contains("native-uid"));
        }
        let text = std::fs::read_to_string(dir.path().join("2026/10/14/log.nljson")).unwrap();
        assert_eq!(text.lines().count(), 5);
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(text.lines().next().unwrap()).unwrap()["level"],
            "DEBUG"
        );
    }
}
#[test]
fn interrupted_log_tail_is_preserved_exactly_or_safely_rejected() {
    use base64::Engine;
    for partial in [
        b"{\"msg\":\"valid\"}".to_vec(),
        b"partial\xff".to_vec(),
        vec![b'x'; 65_537],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let day = dir.path().join("2026/10/14");
        std::fs::create_dir_all(&day).unwrap();
        let path = day.join("log.nljson");
        std::fs::write(&path, &partial).unwrap();
        let writer = Writer::default();
        let mut journal = Journal::new(
            Arc::new(Store::open(dir.path()).unwrap()),
            Some(Box::new(writer.clone())),
            true,
            false,
        );
        let result = journal.event(record(Level::Warn, "next"));
        journal.close().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        if partial.len() > 65_536 {
            assert!(result.is_err());
            assert_eq!(bytes, partial);
            let out = String::from_utf8(writer.0.lock().unwrap().clone()).unwrap();
            assert!(out.contains("log_error"));
        } else {
            result.unwrap();
            let lines: Vec<serde_json::Value> = bytes
                .split(|b| *b == b'\n')
                .filter(|b| !b.is_empty())
                .map(|b| serde_json::from_slice(b).unwrap())
                .collect();
            if partial[0] == b'{' {
                assert_eq!(lines[0]["msg"], "valid");
            } else {
                assert_eq!(
                    base64::engine::general_purpose::STANDARD
                        .decode(lines[0]["partial_tail_base64"].as_str().unwrap())
                        .unwrap(),
                    partial
                );
            }
            assert_eq!(lines.last().unwrap()["msg"], "next");
        }
    }
}
