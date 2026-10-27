use super::Agent;
use crate::{Error, Result};
use std::{
    ffi::OsString,
    os::{fd::OwnedFd, unix::net::UnixStream},
    process::Stdio,
    time::Duration,
};
use tokio::io::AsyncReadExt;
use tokio_util::sync::CancellationToken;
pub(super) const COMMAND_TIMEOUT: Duration = Duration::from_secs(120);
struct ProcessGroup(i32);
impl Drop for ProcessGroup {
    fn drop(&mut self) {
        if self.0 > 0 {
            // SAFETY: this is the dedicated group created for our child; cleanup is best effort.
            unsafe {
                libc::kill(-self.0, libc::SIGKILL);
            }
        }
    }
}
impl Agent {
    pub(super) async fn command(
        &self,
        args: &[OsString],
        token: &CancellationToken,
    ) -> Result<Vec<u8>> {
        if token.is_cancelled() {
            return Err(Error::Cancelled);
        }
        // One native byte stream preserves stdout/stderr write order.
        let (receiver, sender) = UnixStream::pair()?;
        receiver.set_nonblocking(true)?;
        let mut receiver = tokio::net::UnixStream::from_std(receiver)?;
        let mut command = tokio::process::Command::new(&self.control);
        command
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(OwnedFd::from(sender.try_clone()?)))
            .stderr(Stdio::from(OwnedFd::from(sender)))
            .process_group(0)
            .kill_on_drop(true);
        let mut child = command.spawn()?;
        drop(command);
        let mut group = ProcessGroup(child.id().unwrap_or_default() as i32);
        let mut output = Vec::new();
        let operation = async {
            let read = receiver.read_to_end(&mut output);
            tokio::pin!(read);
            let (status, drained) = tokio::select! {status=child.wait()=>(status?,false),result=&mut read=>{result?;(child.wait().await?,true)}};
            // A descendant may retain output descriptors after launchctl exits.
            if !drained {
                tokio::time::timeout(Duration::from_secs(1), &mut read)
                    .await
                    .map_err(|_| {
                        std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            "launchctl output remained open after exit",
                        )
                    })??;
            }
            Ok(status)
        };
        let result = tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),result=tokio::time::timeout(COMMAND_TIMEOUT,operation)=>result.unwrap_or_else(|_|Err(Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut,"launchctl timed out"))))};
        match result {
            Ok(status) => {
                group.0 = 0;
                if status.success() {
                    Ok(output)
                } else {
                    Err(Error::Command {
                        arguments: args
                            .iter()
                            .map(|s| s.to_string_lossy())
                            .collect::<Vec<_>>()
                            .join(" "),
                        status,
                        output,
                    })
                }
            }
            Err(error) => {
                drop(group);
                child.kill().await.ok();
                child.wait().await.ok();
                Err(error)
            }
        }
    }
}
