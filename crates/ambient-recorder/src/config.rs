use crate::{Error, Result, microphone::Preferences};
use fundu::{CustomDurationParser, CustomTimeUnit, TimeUnit};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::Write,
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::{Path, PathBuf},
    sync::LazyLock,
    time::Duration,
};

#[derive(Clone, Debug)]
pub struct Paths {
    config: PathBuf,
    output: PathBuf,
}
impl Paths {
    pub fn new(home: PathBuf, config: Option<PathBuf>, data: Option<PathBuf>) -> Result<Self> {
        let base = |value: Option<PathBuf>, fallback: &str, variable: &str| -> Result<PathBuf> {
            if let Some(value) = value.filter(|p| !p.as_os_str().is_empty()) {
                if !value.is_absolute() {
                    return Err(Error::Invalid(format!(
                        "{variable} must be an absolute path"
                    )));
                }
                Ok(value)
            } else if home.as_os_str().is_empty() {
                Err(Error::Invalid("HOME is required".into()))
            } else {
                Ok(home.join(fallback))
            }
        };
        Ok(Self {
            config: base(config, ".config", "XDG_CONFIG_HOME")?
                .join("ambient-recorder/config.toml"),
            output: base(data, ".local/share", "XDG_DATA_HOME")?.join("ambient-recorder"),
        })
    }
    pub fn from_environment() -> Result<Self> {
        Self::new(
            std::env::var_os("HOME").unwrap_or_default().into(),
            std::env::var_os("XDG_CONFIG_HOME").map(Into::into),
            std::env::var_os("XDG_DATA_HOME").map(Into::into),
        )
    }
    pub fn config(&self) -> PathBuf {
        self.config.clone()
    }
    pub fn output(&self) -> PathBuf {
        self.output.clone()
    }
    pub fn resolve_config(&self, path: Option<&Path>) -> Result<PathBuf> {
        Ok(std::path::absolute(
            path.filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(&self.config),
        )?)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    pub input: Input,
    pub output: Output,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Input {
    pub microphones: Vec<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Output {
    pub dir: PathBuf,
    pub bitrate: i32,
    pub complexity: i32,
    pub sync_interval: String,
}
#[derive(Default)]
pub struct Overrides {
    pub output: Option<PathBuf>,
    pub bitrate: Option<i32>,
    pub complexity: Option<i32>,
    pub sync_interval: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Partial {
    input: Option<PartialInput>,
    output: Option<PartialOutput>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialInput {
    microphones: Option<Vec<String>>,
}
#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct PartialOutput {
    dir: Option<PathBuf>,
    bitrate: Option<i32>,
    complexity: Option<i32>,
    sync_interval: Option<String>,
}

impl Settings {
    pub fn defaults(paths: &Paths) -> Self {
        Self {
            input: Input {
                microphones: vec!["*".into()],
            },
            output: Output {
                dir: paths.output(),
                bitrate: 32000,
                complexity: 2,
                sync_interval: "5s".into(),
            },
        }
    }
    pub fn load(paths: &Paths, path: Option<&Path>) -> Result<(Self, Option<PathBuf>)> {
        let file = paths.resolve_config(path)?;
        let mut settings = Self::defaults(paths);
        let text = match fs::read_to_string(&file) {
            Ok(text) => text,
            Err(e)
                if e.kind() == std::io::ErrorKind::NotFound
                    && path.is_none_or(|p| p.as_os_str().is_empty()) =>
            {
                return Ok((settings, None));
            }
            Err(e) => return Err(e.into()),
        };
        let partial: Partial = toml::from_str(&text)?;
        if let Some(input) = partial.input
            && let Some(microphones) = input.microphones
        {
            settings.input.microphones = microphones;
        }
        if let Some(output) = partial.output {
            settings.apply(Overrides {
                output: output.dir,
                bitrate: output.bitrate,
                complexity: output.complexity,
                sync_interval: output.sync_interval,
            });
        }
        Ok((settings, Some(file)))
    }
    pub fn apply(&mut self, overrides: Overrides) {
        if let Some(dir) = overrides.output {
            self.output.dir = dir;
        }
        if let Some(bitrate) = overrides.bitrate {
            self.output.bitrate = bitrate;
        }
        if let Some(complexity) = overrides.complexity {
            self.output.complexity = complexity;
        }
        if let Some(interval) = overrides.sync_interval {
            self.output.sync_interval = interval;
        }
    }
    pub fn validate(&self) -> Result<Duration> {
        let interval = parse_interval(&self.output.sync_interval)?;
        if self.output.dir.as_os_str().is_empty()
            || !(6000..=128000).contains(&self.output.bitrate)
            || !(0..=10).contains(&self.output.complexity)
        {
            return Err(Error::Invalid("output.dir is required; output.bitrate must be 6000..128000 and output.complexity 0..10".into()));
        }
        Preferences::parse(&self.input.microphones)?;
        Ok(interval)
    }
    pub fn create(paths: &Paths, path: Option<&Path>) -> Result<PathBuf> {
        let path = paths.resolve_config(path)?;
        let text = toml::to_string_pretty(&Self::defaults(paths))?;
        fs::DirBuilder::new().recursive(true).mode(0o700).create(
            path.parent()
                .ok_or_else(|| Error::Invalid("configuration has no parent".into()))?,
        )?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        Ok(path)
    }
}

pub fn parse_interval(value: &str) -> Result<Duration> {
    static GRAMMAR: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(r"\A\+?(?:(?:[0-9]+(?:\.[0-9]*)?|\.[0-9]+)(?:ns|us|µs|μs|ms|s|m|h))+\z")
            .expect("duration grammar")
    });
    static PARSER: LazyLock<CustomDurationParser<'static>> = LazyLock::new(|| {
        CustomDurationParser::builder()
            .time_units(&[
                CustomTimeUnit::with_default(TimeUnit::NanoSecond, &["ns"]),
                CustomTimeUnit::with_default(TimeUnit::MicroSecond, &["us", "µs", "μs"]),
                CustomTimeUnit::with_default(TimeUnit::MilliSecond, &["ms"]),
                CustomTimeUnit::with_default(TimeUnit::Second, &["s"]),
                CustomTimeUnit::with_default(TimeUnit::Minute, &["m"]),
                CustomTimeUnit::with_default(TimeUnit::Hour, &["h"]),
            ])
            .disable_exponent()
            .disable_infinity()
            .parse_multiple(None)
            .build()
    });
    let invalid = || {
        Error::Invalid(format!(
            "output.sync_interval must be a positive Go duration: {value:?}"
        ))
    };
    if !GRAMMAR.is_match(value) {
        return Err(invalid());
    }
    let duration: Duration = PARSER
        .parse(value)
        .map_err(|_| invalid())?
        .try_into()
        .map_err(|_| invalid())?;
    if duration.is_zero() || duration.as_nanos() > i64::MAX as u128 {
        return Err(invalid());
    }
    Ok(duration)
}
