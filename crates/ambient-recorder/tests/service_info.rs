use ambient_recorder::service_info::{Info, Server, query, socket_path};
use std::{fs, os::unix::fs::PermissionsExt};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn socket_lock_uses_exact_native_path_bytes() {
    use ambient_recorder::Error;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    let dir = tempfile::tempdir().unwrap();
    for name in [
        OsString::from("native-é.sock"),
        OsString::from_vec(b"native-\xff.sock".to_vec()),
    ] {
        let socket = dir.path().join(name);
        // APFS rejects non-UTF-8 names; determine support with native file creation.
        let native_error = match fs::write(&socket, b"") {
            Ok(()) => {
                fs::remove_file(&socket).unwrap();
                None
            }
            Err(error) => {
                assert_eq!(error.raw_os_error(), Some(libc::EILSEQ));
                Some(error)
            }
        };
        let result = Server::listen(
            &socket,
            Info {
                version: "native".into(),
                ..Default::default()
            },
        )
        .await;
        if let Some(expected) = native_error {
            let Err(Error::Io(actual)) = result else {
                panic!("server must preserve the native filename error");
            };
            assert_eq!(actual.raw_os_error(), expected.raw_os_error());
        } else {
            let mut server = result.unwrap();
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
            assert!(!socket.exists());
            fs::remove_file(lock).unwrap();
        }
        // Rejected names must not leave a replacement-character lock behind.
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    dir.close().unwrap();
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
    for (status, body) in [
        ("200 OK", "{}".to_string()),
        ("200 OK", "not json".to_string()),
        (
            "200 OK",
            format!("{{\"version\":\"{}\"}}", "x".repeat(4096)),
        ),
        (
            "503 Service Unavailable",
            "{\"version\":\"unhealthy\"}".to_string(),
        ),
    ] {
        let path = dir.path().join("fixture.sock");
        let listener = tokio::net::UnixListener::bind(&path).unwrap();
        let task = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            assert!(stream.read(&mut buf).await.unwrap() > 0);
            let reply = format!(
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            stream.write_all(reply.as_bytes()).await.unwrap();
        });
        let result = query(&path, &CancellationToken::new()).await;
        if status.starts_with("503") {
            assert!(
                matches!(result, Err(ambient_recorder::Error::Invalid(message)) if message.contains("response status"))
            );
        } else {
            assert!(result.is_err());
        }
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

#[tokio::test]
async fn http_methods_and_shutdown_preserve_protocol_and_replacement_socket() {
    let root = tempfile::tempdir().unwrap();
    let socket = root.path().join("identity.sock");
    let mut server = Server::listen(
        &socket,
        Info {
            version: "method-test".into(),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let client = reqwest::Client::builder()
        .unix_socket(socket.clone())
        .no_proxy()
        .timeout(std::time::Duration::from_secs(2))
        .build()
        .unwrap();
    let get = client.get("http://service/version").send().await.unwrap();
    assert_eq!(get.status(), reqwest::StatusCode::OK);
    let length = get.headers()["content-length"].clone();
    let body = get.bytes().await.unwrap();
    assert_eq!(
        length.to_str().unwrap().parse::<usize>().unwrap(),
        body.len()
    );
    assert_eq!(
        serde_json::from_slice::<Info>(&body).unwrap().version,
        "method-test"
    );
    let head = client.head("http://service/version").send().await.unwrap();
    assert_eq!(head.status(), reqwest::StatusCode::OK);
    assert_eq!(head.headers()["content-length"], length);
    assert_eq!(head.headers()["content-type"], "application/json");
    assert!(head.bytes().await.unwrap().is_empty());
    let post = client.post("http://service/version").send().await.unwrap();
    assert_eq!(post.status(), reqwest::StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(post.headers()["allow"], "GET, HEAD");
    assert!(post.bytes().await.unwrap().is_empty());
    let missing = client.get("http://service/missing").send().await.unwrap();
    assert_eq!(missing.status(), reqwest::StatusCode::NOT_FOUND);
    assert!(missing.bytes().await.unwrap().is_empty());
    drop(client);

    fs::rename(&socket, root.path().join("old.sock")).unwrap();
    let replacement = tokio::net::UnixListener::bind(&socket).unwrap();
    server.close().await.unwrap();
    let connection = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let accepted = tokio::time::timeout(std::time::Duration::from_secs(2), replacement.accept())
        .await
        .unwrap()
        .unwrap();
    drop((connection, accepted, replacement));
    root.close().unwrap();
}
