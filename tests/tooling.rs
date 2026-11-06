#![cfg(unix)]

use std::{fs, path::Path, process::Command};

#[test]
fn cargo_wrappers_preserve_native_socket_scratch_and_child_status() -> std::io::Result<()> {
    let temp = tempfile::tempdir()?;
    let project = temp.path().join("long-checkout-component-".repeat(8));
    fs::create_dir(&project)?;
    fs::write(
        project.join("Cargo.toml"),
        "[package]\nname = 'scratch-probe'\nversion = '0.0.0'\nedition = '2024'\n\
         [[bin]]\nname = 'scratch-probe'\npath = 'main.rs'\n\n[workspace]\n",
    )?;
    fs::write(
        project.join("main.rs"),
        r#"use std::{env, fs, os::unix::net::UnixListener};
fn main() -> std::io::Result<()> {
    let mut args = env::args_os().skip(1);
    let socket = env::temp_dir().join(args.next().unwrap()).join("probe.sock");
    let listener = UnixListener::bind(&socket)?;
    drop(listener);
    fs::remove_file(socket)?;
    if args.next().is_some() {
        std::process::exit(17);
    }
    Ok(())
}
"#,
    )?;

    for wrapper in [
        "scripts/cargo.sh",
        "crates/ambient-recorder/scripts/cargo.sh",
    ] {
        for failure in [false, true] {
            let mut command = Command::new("bash");
            command
                .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join(wrapper))
                .args(["run", "--offline", "--quiet", "--target-dir"])
                .arg(project.join("target"))
                .arg("--")
                .arg(temp.path().file_name().unwrap())
                .current_dir(&project);
            if failure {
                command.arg("fail");
            }
            let output = command.output()?;
            assert_eq!(
                output.status.code(),
                Some(if failure { 17 } else { 0 }),
                "{wrapper}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            assert!(!temp.path().join("probe.sock").exists());
            assert!(!project.join(".tmp").exists());
        }
    }
    let path = temp.path().to_owned();
    temp.close()?;
    assert!(!path.exists());
    Ok(())
}
