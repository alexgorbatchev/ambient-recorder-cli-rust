use ambient_recorder::storage::{Store, current_file};
use chrono::{TimeZone, Utc};
use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
};

#[test]
fn native_locks_and_root_confinement_reject_stale_or_foreign_current_files() {
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("audio");
    assert!(current_file(&output).is_err());
    assert!(!output.exists());
    let store = Store::open(&output).unwrap();
    let lock = store.lock().unwrap();
    assert!(Store::open(&output).unwrap().lock().is_err());
    let at = Utc.with_ymd_and_hms(2026, 10, 14, 12, 30, 0).unwrap();
    let mut segment = store.create_segment(at).unwrap();
    segment.file.write_all(b"OggS").unwrap();
    store.mark_current(&segment).unwrap();
    assert_eq!(
        current_file(&output).unwrap(),
        output.join(&segment.relative)
    );
    for _ in 0..32 {
        assert_eq!(
            current_file(&output).unwrap(),
            output.join(&segment.relative)
        );
    }
    assert_eq!(
        segment.file.metadata().unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert_eq!(
        fs::metadata(output.join("2026/10/14"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o700
    );
    let other = store.create_segment(at).unwrap();
    assert_ne!(other.relative, segment.relative);
    assert_eq!(fs::read(output.join(&segment.relative)).unwrap(), b"OggS");
    drop(segment);
    assert!(current_file(&output).is_err());
    store.clear_current().unwrap();
    store.clear_current().unwrap();
    symlink("../outside", output.join(".recording.current")).unwrap();
    assert!(current_file(&output).is_err());
    store.clear_current().unwrap();
    symlink(dir.path(), output.join("2025")).unwrap();
    assert!(
        store
            .create_segment(Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap())
            .is_err()
    );
    drop(lock);
    assert!(store.lock().is_ok());
}

#[test]
fn logs_append_and_queries_cannot_block_on_fifo() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path()).unwrap();
    let at = Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0).unwrap();
    store.open_log(at).unwrap().write_all(b"first\n").unwrap();
    store.open_log(at).unwrap().write_all(b"second\n").unwrap();
    assert_eq!(
        fs::read(dir.path().join("2026/01/01/log.nljson")).unwrap(),
        b"first\nsecond\n"
    );
    let fifo = dir.path().join("fifo");
    let path = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    symlink("fifo", dir.path().join(".recording.current")).unwrap();
    assert!(current_file(dir.path()).is_err());
    assert!(Store::open("").is_err());
}
