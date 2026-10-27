use ambient_recorder::service_info::{Info, Server, query, socket_path};
use std::{fs, os::unix::fs::PermissionsExt};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn socket_lock_uses_exact_native_path_bytes() {
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    let socket = dir
        .path()
        .join(std::ffi::OsString::from_vec(b"native-\xff.sock".to_vec()));
    let mut server = Server::listen(
        &socket,
        Info {
            version: "native".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let mut lock = socket.as_os_str().to_os_string();
    lock.push(".lock");
    assert!(std::path::Path::new(&lock).exists());
    assert_eq!(
        query(&socket, &CancellationToken::new())
            .await
            .unwrap()
            .version,
        "native"
    );
    server.close().await.unwrap();
}

#[tokio::test]
async fn native_unix_http_identity_is_private_exclusive_and_snapshot_owned() {
    let dir = tempfile::tempdir().unwrap();
    let socket = socket_path(&dir.path().join("agent.plist"));
    let token = CancellationToken::new();
    let mut server = Server::listen(
        &socket,
        Info {
            version: "rust-0.1.0".into(),
            output_dir: "/absolute/audio".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        fs::metadata(&socket).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        Server::listen(
            &socket,
            Info {
                version: "other".into(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    for i in 0..12 {
        server
            .state()
            .set_current_file(format!("/absolute/audio/{i}.opus"));
        server.state().set_microphone(format!("USB {i}"));
        let info = query(&socket, &token).await.unwrap();
        assert_eq!(info.version, "rust-0.1.0");
        assert_eq!(info.current_file, format!("/absolute/audio/{i}.opus"));
        assert_eq!(info.microphone, Some(format!("USB {i}")));
    }
    let idle = tokio::net::UnixStream::connect(&socket).await.unwrap();
    server.close().await.unwrap();
    server.close().await.unwrap();
    drop(idle);
    assert!(!socket.exists());
    assert!(socket.with_extension("sock.lock").exists());
    assert!(query(&socket, &token).await.is_err());
    assert!(Server::listen(&socket, Info::default()).await.is_err());
    fs::write(&socket, b"preserve").unwrap();
    assert!(
        Server::listen(
            &socket,
            Info {
                version: "x".into(),
                ..Default::default()
            }
        )
        .await
        .is_err()
    );
    assert_eq!(fs::read(&socket).unwrap(), b"preserve");
}

#[tokio::test]
async fn native_client_rejects_invalid_oversized_and_delayed_responses_and_cancels() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let dir = tempfile::tempdir().unwrap();
    for body in [
        "{}".to_string(),
        "not json".to_string(),
        format!("{{\"version\":\"{}\"}}", "x".repeat(4096)),
    ] {
        let path = dir.path().join("fixture.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            assert!(stream.read(&mut buf).await.unwrap() > 0);
            let reply = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        assert!(query(&path, &CancellationToken::new()).await.is_err());
        task.await.unwrap();
        fs::remove_file(path).unwrap();
    }
    let path = dir.path().join("delayed.sock");
    let listener = tokio::net::UnixListener::bind(&path).unwrap();
    let token = CancellationToken::new();
    let task = tokio::spawn({
        let token = token.clone();
        async move {
            let _ = listener.accept().await.unwrap();
            token.cancel();
        }
    });
    assert!(matches!(
        query(&path, &token).await,
        Err(ambient_recorder::Error::Cancelled)
    ));
    task.await.unwrap();
}
