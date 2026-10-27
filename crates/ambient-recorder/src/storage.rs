use crate::{Error, Result};
use cap_fs_ext::OpenOptionsSyncExt;
use cap_std::{
    ambient_authority,
    fs::{Dir, DirBuilder, DirBuilderExt, OpenOptions, OpenOptionsExt},
};
use chrono::{DateTime, Local, TimeZone};
use std::{
    fs::{self, File, TryLockError},
    os::unix::fs::{DirBuilderExt as _, MetadataExt},
    path::{Component, Path, PathBuf},
};

pub struct Store {
    root: Dir,
    path: PathBuf,
}
pub struct Segment {
    pub file: File,
    pub relative: PathBuf,
    root: PathBuf,
}
impl Store {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        if path.as_ref().as_os_str().is_empty() {
            return Err(Error::Invalid("recording directory is empty".into()));
        }
        let path = std::path::absolute(path)?;
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&path)?;
        Ok(Self {
            root: Dir::open_ambient_dir(&path, ambient_authority())?,
            path,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    fn day<Tz: TimeZone>(&self, at: &DateTime<Tz>) -> Result<PathBuf>
    where
        Tz::Offset: std::fmt::Display,
    {
        let path = PathBuf::from(at.format("%Y/%m/%d").to_string());
        self.root
            .create_dir_with(&path, DirBuilder::new().recursive(true).mode(0o700))?;
        Ok(path)
    }
    pub fn create_segment<Tz: TimeZone>(&self, mut at: DateTime<Tz>) -> Result<Segment>
    where
        Tz::Offset: std::fmt::Display,
    {
        loop {
            let relative = self
                .day(&at)?
                .join(at.format("%H-%M-%S.%9f.opus").to_string());
            match self.root.open_with(
                &relative,
                OpenOptions::new().write(true).create_new(true).mode(0o600),
            ) {
                Ok(file) => {
                    return Ok(Segment {
                        file: file.into_std(),
                        relative,
                        root: self.path.clone(),
                    });
                }
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    at = Local::now().with_timezone(&at.timezone())
                }
                Err(e) => return Err(e.into()),
            }
        }
    }
    pub fn open_log<Tz: TimeZone>(&self, at: DateTime<Tz>) -> Result<File>
    where
        Tz::Offset: std::fmt::Display,
    {
        Ok(self
            .root
            .open_with(
                self.day(&at)?.join("log.nljson"),
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .append(true)
                    .mode(0o600),
            )?
            .into_std())
    }
    pub fn lock(&self) -> Result<File> {
        let file = self
            .root
            .open_with(
                ".recording.lock",
                OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .mode(0o600),
            )?
            .into_std();
        file.try_lock().map_err(lock_error)?;
        Ok(file)
    }
    pub fn mark_current(&self, segment: &Segment) -> Result<()> {
        if segment.root != self.path || !is_local(&segment.relative) {
            return Err(Error::Invalid(
                "recording segment belongs to another storage directory".into(),
            ));
        }
        let on_disk = self.root.open(&segment.relative)?.into_std().metadata()?;
        let opened = segment.file.metadata()?;
        if on_disk.dev() != opened.dev() || on_disk.ino() != opened.ino() || !opened.is_file() {
            return Err(Error::Invalid(
                "recording segment no longer identifies its reserved regular file".into(),
            ));
        }
        segment.file.try_lock().map_err(lock_error)?;
        self.clear_current()?;
        self.root.symlink(&segment.relative, ".recording.current")?;
        Ok(())
    }
    pub fn clear_current(&self) -> Result<()> {
        match self.root.remove_file(".recording.current") {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}
fn is_local(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && path
            .components()
            .all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}
fn lock_error(error: TryLockError) -> Error {
    match error {
        TryLockError::WouldBlock => Error::Io(std::io::Error::new(
            std::io::ErrorKind::WouldBlock,
            "another recorder may be active",
        )),
        TryLockError::Error(e) => Error::Io(e),
    }
}
fn current_error(error: std::io::Error) -> Error {
    if error.kind() == std::io::ErrorKind::NotFound {
        Error::NoCurrentFile
    } else {
        Error::Io(error)
    }
}
pub fn current_file(directory: impl AsRef<Path>) -> Result<PathBuf> {
    let directory = std::path::absolute(directory)?;
    let root = Dir::open_ambient_dir(&directory, ambient_authority()).map_err(current_error)?;
    let target = root
        .read_link_contents(".recording.current")
        .map_err(current_error)?;
    if !is_local(&target) {
        return Err(Error::Invalid(
            "current recording pointer is outside its storage directory".into(),
        ));
    }
    let file = root
        .open_with(&target, OpenOptions::new().read(true).nonblock(true))
        .map_err(current_error)?
        .into_std();
    if !file.metadata()?.is_file() {
        return Err(Error::Invalid(
            "current recording pointer does not reference a regular file".into(),
        ));
    }
    match file.try_lock_shared() {
        Err(TryLockError::WouldBlock) => Ok(directory.join(target)),
        Ok(()) => Err(Error::NoCurrentFile),
        Err(TryLockError::Error(e)) => Err(e.into()),
    }
}
