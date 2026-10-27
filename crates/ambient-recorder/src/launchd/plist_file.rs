use super::Agent;
use crate::{Error, Result, service_info};
use std::{
    fs::{self, OpenOptions},
    os::unix::fs::{DirBuilderExt, OpenOptionsExt},
    path::Path,
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;

pub(super) fn path_value(path: &Path) -> Result<plist::Value> {
    path.to_str()
        .map(|s| s.into())
        .ok_or_else(|| Error::Invalid("launchd property-list paths must be valid UTF-8".into()))
}
pub(super) fn create_directory(path: &Path) -> Result<()> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    Ok(())
}
pub(super) fn create(path: &Path, value: &plist::Value) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    let write = (|| {
        value.to_writer_xml(&mut file)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write {
        drop(file);
        return match fs::remove_file(path) {
            Ok(()) => Err(error),
            Err(cleanup) => Err(Error::Multiple(vec![error, cleanup.into()])),
        };
    }
    Ok(())
}
impl Agent {
    pub(super) async fn read_configuration(
        &self,
        token: &CancellationToken,
    ) -> Result<plist::Value> {
        let read = async {
            if !self.installed()? {
                return Err(Error::Invalid("service is not installed".into()));
            }
            let mut file = tokio::fs::File::open(&self.plist).await?;
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes).await?;
            Ok(plist::Value::from_reader(std::io::Cursor::new(bytes))?)
        };
        tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),result=tokio::time::timeout(std::time::Duration::from_secs(1),read)=>result.map_err(|_|std::io::Error::new(std::io::ErrorKind::TimedOut,"read service configuration timed out"))?}
    }
    pub async fn refresh(&self, token: &CancellationToken) -> Result<bool> {
        let mut value = self.read_configuration(token).await?;
        let fields = value
            .as_dictionary_mut()
            .ok_or_else(|| Error::Invalid("service configuration must be a dictionary".into()))?;
        let args=fields.get_mut("ProgramArguments").and_then(plist::Value::as_array_mut).filter(|a|!a.is_empty()&&a.iter().all(|v|v.as_string().is_some())).ok_or_else(||Error::Invalid("service has no valid executable arguments; reinstall with service uninstall and service install".into()))?;
        let binary = path_value(&self.binary)?;
        let mut changed = args[0] != binary;
        args[0] = binary.clone();
        if let Some(program) = fields.get_mut("Program") {
            changed |= *program != binary;
            *program = binary;
        }
        if !fields.contains_key("EnvironmentVariables") {
            fields.insert(
                "EnvironmentVariables".into(),
                plist::Value::Dictionary(plist::Dictionary::new()),
            );
        }
        let env = fields
            .get_mut("EnvironmentVariables")
            .and_then(plist::Value::as_dictionary_mut)
            .ok_or_else(|| Error::Invalid("service environment must be a dictionary".into()))?;
        let socket = path_value(&service_info::socket_path(&self.plist))?;
        changed |= env.get(service_info::ENVIRONMENT) != Some(&socket);
        env.insert(service_info::ENVIRONMENT.into(), socket);
        if changed {
            let mut temporary = tempfile::Builder::new()
                .prefix(".ambient-recorder-")
                .suffix(".plist")
                .tempfile_in(self.plist.parent().expect("absolute service file parent"))?;
            value.to_writer_xml(temporary.as_file_mut())?;
            temporary.as_file().sync_all()?;
            temporary
                .persist(&self.plist)
                .map_err(|e| Error::Io(e.error))?;
        }
        Ok(changed)
    }
}
