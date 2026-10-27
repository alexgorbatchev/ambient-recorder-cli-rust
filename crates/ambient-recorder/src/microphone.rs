use crate::{Error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub id: u32,
    pub name: String,
    pub uid: String,
    pub default: bool,
    pub transport: String,
    pub terminal: String,
    pub manufacturer: String,
    pub model: String,
}
impl Device {
    pub fn new(id: u32, name: impl Into<String>, uid: impl Into<String>, default: bool) -> Self {
        Self {
            id,
            name: name.into(),
            uid: uid.into(),
            default,
            transport: String::new(),
            terminal: String::new(),
            manufacturer: String::new(),
            model: String::new(),
        }
    }
}

#[derive(Debug)]
enum Matcher {
    Uid(String),
    Name(regex::Regex),
}
#[derive(Debug)]
struct Selector {
    exclude: bool,
    matcher: Matcher,
}
impl Selector {
    fn matches(&self, device: &Device) -> bool {
        match &self.matcher {
            Matcher::Uid(uid) => *uid == device.uid,
            Matcher::Name(pattern) => pattern.is_match(&device.name),
        }
    }
}
#[derive(Debug)]
pub struct Preferences {
    selectors: Vec<Selector>,
}
impl Preferences {
    pub fn parse(values: &[String]) -> Result<Self> {
        let selectors=values.iter().map(|raw|{
            let (exclude,value)=raw.strip_prefix('!').map_or((false,raw.as_str()),|v|(true,v));
            let uid=value.strip_prefix("uid:");
            let value=uid.unwrap_or(value);
            if value.trim().is_empty() {return Err(Error::Invalid(format!("microphone preference {raw:?}: provide a name pattern or uid:<UID>, optionally prefixed with !")));}
            let matcher=if uid.is_some() {Matcher::Uid(value.into())} else {
                let pattern=value.split('*').map(regex::escape).collect::<Vec<_>>().join(".*");
                Matcher::Name(regex::Regex::new(&format!("(?s)\\A{pattern}\\z")).map_err(|e|Error::Invalid(e.to_string()))?)
            };
            Ok(Selector{exclude,matcher})
        }).collect::<Result<Vec<_>>>()?;
        Ok(Self { selectors })
    }
    pub fn ordered<'a>(&self, devices: &'a [Device]) -> Vec<&'a Device> {
        let mut eligible: Vec<_> = devices
            .iter()
            .filter(|device| {
                !self
                    .selectors
                    .iter()
                    .any(|s| s.exclude && s.matches(device))
            })
            .collect();
        eligible.sort_by(|a, b| {
            b.default
                .cmp(&a.default)
                .then_with(|| a.uid.cmp(&b.uid))
                .then_with(|| a.id.cmp(&b.id))
        });
        let mut result = Vec::with_capacity(eligible.len());
        for selector in self.selectors.iter().filter(|s| !s.exclude) {
            for device in &eligible {
                if selector.matches(device) && !result.iter().any(|d: &&Device| d.id == device.id) {
                    result.push(*device);
                }
            }
        }
        for device in eligible {
            if !result.iter().any(|d| d.id == device.id) {
                result.push(device);
            }
        }
        result
    }
}
