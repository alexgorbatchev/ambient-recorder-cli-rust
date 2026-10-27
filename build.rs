fn main() {
    println!("cargo::rerun-if-changed=Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let path = std::path::Path::new(
            &std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"),
        )
        .join("Info.plist");
        // Pass separate linker arguments so native paths containing commas remain intact.
        for argument in ["-Wl,-sectcreate", "-Wl,__TEXT", "-Wl,__info_plist"] {
            println!("cargo::rustc-link-arg-bin=ambient-recorder={argument}");
        }
        println!(
            "cargo::rustc-link-arg-bin=ambient-recorder={}",
            path.display()
        );
    }
}
