use crate::{Error, Result};
use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode,
    body::{Bytes, Incoming},
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    convert::Infallible,
    fs::{self, File, OpenOptions},
    os::unix::fs::{FileTypeExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
    time::Duration,
};
use tokio::{
    net::UnixListener,
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;

pub const ENVIRONMENT: &str = "AMBIENT_RECORDER_SERVICE_SOCKET";
const TIMEOUT: Duration = Duration::from_secs(1);
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Info {
    pub version: String,
    pub output_dir: String,
    pub config_path: String,
    pub current_file: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub microphone: Option<String>,
}
pub fn socket_path(plist: &Path) -> PathBuf {
    let hash = Sha256::digest(plist.file_name().unwrap_or_default().as_encoded_bytes());
    let name = format!(
        ".ar-{:02x}{:02x}{:02x}{:02x}.sock",
        hash[0], hash[1], hash[2], hash[3]
    );
    plist.with_file_name(name)
}
#[derive(Clone)]
pub struct State(Arc<RwLock<Info>>);
impl State {
    pub fn snapshot(&self) -> Info {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
    pub fn set_current_file(&self, path: impl Into<String>) {
        self.0
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .current_file = path.into();
    }
    pub fn set_microphone(&self, name: impl Into<String>) {
        self.0.write().unwrap_or_else(|e| e.into_inner()).microphone = Some(name.into());
    }
}
struct Endpoint {
    path: PathBuf,
    identity: (u64, u64),
    _lock: File,
}
impl Endpoint {
    fn remove(&self) -> std::io::Result<()> {
        match fs::symlink_metadata(&self.path) {
            Ok(meta) if (meta.dev(), meta.ino()) == self.identity => fs::remove_file(&self.path),
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}
impl Drop for Endpoint {
    fn drop(&mut self) {
        let _ = self.remove();
    }
}
pub struct Server {
    state: State,
    shutdown: CancellationToken,
    task: Option<JoinHandle<Result<()>>>,
}
impl Server {
    pub async fn listen(path: &Path, info: Info) -> Result<Self> {
        if info.version.is_empty() {
            return Err(Error::Invalid("service version is empty".into()));
        }
        let path = std::path::absolute(path)?;
        let mut lock_path = path.as_os_str().to_os_string();
        lock_path.push(".lock");
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(PathBuf::from(lock_path))?;
        lock.try_lock()
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::WouldBlock, e))?;
        match fs::symlink_metadata(&path) {
            Ok(meta) if meta.file_type().is_socket() => fs::remove_file(&path)?,
            Ok(_) => {
                return Err(Error::Invalid(
                    "service identity path is not a socket".into(),
                ));
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        let listener = UnixListener::bind(&path)?;
        let meta = fs::symlink_metadata(&path)?;
        let endpoint = Endpoint {
            path: path.clone(),
            identity: (meta.dev(), meta.ino()),
            _lock: lock,
        };
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let state = State(Arc::new(RwLock::new(info)));
        let shutdown = CancellationToken::new();
        let task = tokio::spawn(serve(listener, endpoint, state.clone(), shutdown.clone()));
        Ok(Self {
            state,
            shutdown,
            task: Some(task),
        })
    }
    pub fn state(&self) -> State {
        self.state.clone()
    }
    pub async fn close(&mut self) -> Result<()> {
        self.shutdown.cancel();
        if let Some(task) = self.task.take() {
            task.await
                .map_err(|e| Error::Invalid(format!("service identity task failed: {e}")))??;
        }
        Ok(())
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.shutdown.cancel();
    }
}
async fn serve(
    listener: UnixListener,
    endpoint: Endpoint,
    state: State,
    shutdown: CancellationToken,
) -> Result<()> {
    let mut connections = JoinSet::new();
    let result = loop {
        tokio::select! {biased;
            _=shutdown.cancelled()=>break Ok(()),
            Some(_)=connections.join_next(),if !connections.is_empty()=>{},
            accepted=listener.accept()=>match accepted {
                Ok((stream,_))=>{
                    let state=state.clone();
                    let shutdown=shutdown.clone();
                    connections.spawn(async move {
                        let mut io=tokio_io_timeout::TimeoutStream::new(stream);
                        io.set_read_timeout(Some(TIMEOUT));
                        io.set_write_timeout(Some(TIMEOUT));
                        let io=TokioIo::new(Box::pin(io));
                        let mut builder=hyper::server::conn::http1::Builder::new();
                        builder.timer(TokioTimer::new()).header_read_timeout(TIMEOUT);
                        let service=service_fn(move |request|respond(request,state.clone()));
                        tokio::select! {_=shutdown.cancelled()=>{},_=builder.serve_connection(io,service)=>{}}
                    });
                },
                Err(e)=>break Err(Error::Io(e)),
            }
        }
    };
    shutdown.cancel();
    while connections.join_next().await.is_some() {}
    drop(listener);
    let cleanup = endpoint.remove().map_err(Error::Io);
    result.and(cleanup)
}
async fn respond(
    request: Request<Incoming>,
    state: State,
) -> std::result::Result<Response<Full<Bytes>>, Infallible> {
    let mut response = Response::new(Full::new(Bytes::new()));
    if request.uri().path() != "/version" {
        *response.status_mut() = StatusCode::NOT_FOUND;
    } else if request.method() != Method::GET && request.method() != Method::HEAD {
        *response.status_mut() = StatusCode::METHOD_NOT_ALLOWED;
        response.headers_mut().insert(
            "allow",
            hyper::header::HeaderValue::from_static("GET, HEAD"),
        );
    } else {
        let mut body =
            serde_json::to_vec(&state.snapshot()).expect("string-only identity serializes");
        body.push(b'\n');
        response.headers_mut().insert(
            "content-type",
            hyper::header::HeaderValue::from_static("application/json"),
        );
        response.headers_mut().insert(
            "content-length",
            hyper::header::HeaderValue::from(body.len()),
        );
        if request.method() != Method::HEAD {
            *response.body_mut() = Full::new(Bytes::from(body));
        }
    }
    Ok(response)
}
pub async fn query(path: &Path, token: &CancellationToken) -> Result<Info> {
    let operation = async {
        let client = reqwest::Client::builder()
            .unix_socket(path.to_path_buf())
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(TIMEOUT)
            .build()?;
        let mut response = client.get("http://service/version").send().await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Invalid(
                "invalid service identity response status".into(),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if chunk.len() > 4096 - bytes.len() {
                return Err(Error::Invalid("service identity exceeds 4096 bytes".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        let info: Info = serde_json::from_slice(&bytes)?;
        if info.version.is_empty() {
            return Err(Error::Invalid("service version is empty".into()));
        }
        Ok(info)
    };
    tokio::select! {biased;_=token.cancelled()=>Err(Error::Cancelled),result=tokio::time::timeout(TIMEOUT,operation)=>result.map_err(|_|Error::Io(std::io::Error::new(std::io::ErrorKind::TimedOut,"service identity query timed out")))?}
}
