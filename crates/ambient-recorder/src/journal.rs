//! Daily durable diagnostics with bounded interrupted-record recovery.
use crate::{Error, Result, storage::Store};
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::Serialize;
use serde_json::{Map, Value};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    sync::Arc,
};

const MAX_TAIL: u64 = 64 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Level {
    Debug,
    Info,
    Warn,
    Error,
}
#[derive(Clone, Debug, Serialize)]
pub struct Record {
    time: DateTime<Utc>,
    level: Level,
    msg: String,
    #[serde(flatten)]
    fields: Map<String, Value>,
}
impl Record {
    pub fn new(
        time: DateTime<Utc>,
        level: Level,
        message: impl Into<String>,
        fields: Value,
    ) -> Result<Self> {
        let Value::Object(fields) = fields else {
            return Err(Error::Invalid("diagnostic fields must be an object".into()));
        };
        if ["time", "level", "msg"]
            .iter()
            .any(|key| fields.contains_key(*key))
        {
            return Err(Error::Invalid(
                "diagnostic fields cannot replace time, level, or msg".into(),
            ));
        }
        Ok(Self {
            time,
            level,
            msg: message.into(),
            fields,
        })
    }
    pub fn now(level: Level, message: impl Into<String>, fields: Value) -> Result<Self> {
        Self::new(Utc::now(), level, message, fields)
    }
}

pub struct Journal {
    store: Arc<Store>,
    file: Option<File>,
    day: String,
    console: Option<Box<dyn Write>>,
    agent: bool,
    color: bool,
}
impl Journal {
    pub fn new(
        store: Arc<Store>,
        console: Option<Box<dyn Write>>,
        agent: bool,
        color: bool,
    ) -> Self {
        Self {
            store,
            file: None,
            day: String::new(),
            console,
            agent,
            color,
        }
    }
    fn record(&mut self, record: &Record) -> Result<()> {
        let day = record
            .time
            .with_timezone(&chrono::Local)
            .format("%Y/%m/%d")
            .to_string();
        if self.file.is_none() || day != self.day {
            self.close()?;
            self.file = Some(
                self.store
                    .open_log(record.time.with_timezone(&chrono::Local))?,
            );
            self.day = day;
            if let Err(error) = self.repair_tail(record.time) {
                self.close()?;
                return Err(error);
            }
        }
        write_record(self.file.as_mut().expect("opened log"), record)
    }
    fn repair_tail(&mut self, at: DateTime<Utc>) -> Result<()> {
        let file = self.file.as_mut().expect("opened log");
        let size = file.metadata()?.len();
        if size == 0 {
            return Ok(());
        }
        let start = size.saturating_sub(MAX_TAIL);
        let mut tail = vec![0; (size - start) as usize];
        file.seek(SeekFrom::Start(start))?;
        file.read_exact(&mut tail)?;
        if tail.last() == Some(&b'\n') {
            return Ok(());
        }
        let last = tail.iter().rposition(|byte| *byte == b'\n');
        if last.is_none() && start != 0 {
            return Err(Error::Invalid(
                "incomplete diagnostic log tail exceeds 64 KiB; preserving file and using stderr"
                    .into(),
            ));
        }
        let partial_offset = last.map_or(0, |i| i + 1);
        let partial = &tail[partial_offset..];
        if serde_json::from_slice::<Value>(partial).is_ok() {
            file.write_all(b"\n")?;
            return Ok(());
        }
        file.set_len(start + partial_offset as u64)?;
        let fields = match std::str::from_utf8(partial) {
            Ok(text) => serde_json::json!({"partial_tail": text}),
            Err(_) => {
                serde_json::json!({"partial_tail_base64": base64::engine::general_purpose::STANDARD.encode(partial)})
            }
        };
        let recovered = Record::new(
            at,
            Level::Warn,
            "Recovered incomplete diagnostic record",
            fields,
        )?;
        write_record(file, &recovered)?;
        file.sync_all()?;
        Ok(())
    }
    /// Console failure does not interrupt the daily diagnostic stream. A daily
    /// write failure appears once on the console as the event's log_error field.
    pub fn event(&mut self, mut record: Record) -> Result<()> {
        let result = self.record(&record);
        if let Err(error) = &result {
            record
                .fields
                .insert("log_error".into(), error.to_string().into());
        }
        if record.level >= Level::Info || result.is_err() {
            let skip = !self.agent && result.is_ok() && record.msg == "Microphone available";
            if !skip {
                let bytes = if self.agent {
                    encode_record(&record)
                } else {
                    Ok(human_record(&record, self.color).into_bytes())
                };
                if let (Some(console), Ok(bytes)) = (self.console.as_mut(), bytes) {
                    let _ = console.write_all(&bytes);
                }
            }
        }
        result
    }
    pub fn sync(&self) -> Result<()> {
        if let Some(file) = &self.file {
            file.sync_all()?;
        }
        Ok(())
    }
    pub fn close(&mut self) -> Result<()> {
        if let Some(file) = self.file.take() {
            file.sync_all()?;
        }
        Ok(())
    }
}
impl Drop for Journal {
    fn drop(&mut self) {
        let _ = self.close();
    }
}
fn encode_record(record: &Record) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(record)?;
    bytes.push(b'\n');
    Ok(bytes)
}
fn write_record(writer: &mut impl Write, record: &Record) -> Result<()> {
    writer.write_all(&encode_record(record)?)?;
    Ok(())
}
fn safe_text(text: &str, value: bool) -> String {
    if text
        .chars()
        .any(|c| c.is_control() || matches!(c, '\u{2028}' | '\u{2029}'))
        || (value && (text.chars().any(char::is_whitespace) || text.contains(['"', '='])))
    {
        serde_json::to_string(text)
            .expect("string serializes")
            .replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    } else {
        text.into()
    }
}
fn human_record(record: &Record, color: bool) -> String {
    let tag = match record.level {
        Level::Debug => "DBG",
        Level::Info => "INF",
        Level::Warn => "WRN",
        Level::Error => "ERR",
    };
    let tag = if color {
        use anstyle::{AnsiColor, Color, Style};
        let foreground = match record.level {
            Level::Debug => AnsiColor::Cyan,
            Level::Info => AnsiColor::Green,
            Level::Warn => AnsiColor::Yellow,
            Level::Error => AnsiColor::Red,
        };
        let style = Style::new().fg_color(Some(Color::Ansi(foreground)));
        format!("{style}{tag}{style:#}")
    } else {
        tag.into()
    };
    let mut line = format!(
        "{} {tag} {}",
        record
            .time
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%d %H:%M:%S%.3f %z"),
        safe_text(&record.msg, false)
    );
    for (field, name) in [
        ("microphone_name", "microphone"),
        ("microphone_transport", "connection"),
        ("previous_microphone_name", "previous_microphone"),
        ("output_directory", "output"),
        ("playback", "playback"),
        ("path", "file"),
        ("error", "error"),
        ("log_error", "log_error"),
        ("retry_in", "retry_in"),
        ("dropped_frames", "dropped_frames"),
        ("clock_shift", "clock_shift"),
    ] {
        let Some(value) = record
            .fields
            .get(field)
            .filter(|v| !v.is_null() && v.as_str() != Some(""))
        else {
            continue;
        };
        let text = if field == "playback" && value == "system" {
            "all applications".into()
        } else {
            value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned)
        };
        line.push_str(&format!(
            " {name}={}",
            safe_text(&cli_command_kit::display_message(&text), true)
        ));
    }
    line.push('\n');
    line
}
