use crate::Result;
use ambient_recorder::{
    launchd::{Agent, ServiceStatus},
    microphone::Device,
};
use cli_command_kit::Mode;
use std::{io::Write, path::Path};
use tabled::{
    builder::Builder,
    settings::{Padding, Style},
};
pub(super) fn fold(value: &str, home: &Path) -> String {
    cli_command_kit::fold_message(value, home)
}
pub(super) fn microphones(
    out: &mut dyn Write,
    devices: &[Device],
    mode: Mode,
    home: &Path,
) -> Result<()> {
    if devices.is_empty() {
        writeln!(out, "No microphones available")?;
    }
    for d in devices {
        let name = fold(&d.name, home);
        let uid = fold(&d.uid, home);
        if mode == Mode::Agent {
            writeln!(
                out,
                "name={name:?} uid={uid:?} device_id={} default={} transport={:?} type={:?} manufacturer={:?} model={:?}",
                d.id,
                d.default,
                fold(&d.transport, home),
                fold(&d.terminal, home),
                fold(&d.manufacturer, home),
                fold(&d.model, home)
            )?;
        } else {
            writeln!(
                out,
                "{name:?}{}\n  Connection: {}\n  Input type: {}",
                if d.default { " [system default]" } else { "" },
                fold(&d.transport, home),
                fold(&d.terminal, home)
            )?;
            for (label, value) in [("Manufacturer", &d.manufacturer), ("Model", &d.model)] {
                if !value.is_empty() {
                    writeln!(out, "  {label}: {}", fold(value, home))?;
                }
            }
            if !uid.is_empty() {
                writeln!(out, "  Preference: {:?}", format!("uid:{uid}"))?;
            }
            writeln!(out)?;
        }
    }
    Ok(())
}
pub(super) fn service_result(
    out: &mut dyn Write,
    action: &str,
    agent: &Agent,
    mode: Mode,
    home: &Path,
) -> Result<()> {
    let path = cli_command_kit::fold_home(&agent.plist, home);
    if mode == Mode::Agent {
        writeln!(out, "service={action} plist={path:?}")?;
    } else {
        let message = match action {
            "install" => "Service installed. Starts at login; recording launch requested.",
            "start" => "Service enabled at login; recording launch requested.",
            "stop" => "Service stopped. Startup at login disabled.",
            "restart" => "Service restarted. Startup at login enabled; recording launch requested.",
            "uninstall" => "Service uninstalled. Recordings and configuration retained.",
            _ => unreachable!("parsed service action"),
        };
        writeln!(out, "{message}\nService file: {path}")?;
    }
    Ok(())
}
pub(super) fn status(
    out: &mut dyn Write,
    status: &ServiceStatus,
    agent: &Agent,
    mode: Mode,
    home: &Path,
    details: bool,
) -> Result<()> {
    let s = status;
    let plist = cli_command_kit::fold_home(&agent.plist, home);
    let output = fold(&s.paths.output, home);
    let config = fold(&s.paths.config, home);
    let current = fold(&s.paths.current, home);
    let mic = fold(&s.microphone, home);
    let version = fold(&s.service_version, home);
    if mode == Mode::Agent {
        writeln!(
            out,
            "installed={} loaded={} cli_version={:?} service_version={version:?} plist={plist:?} output_dir={output:?} config_path={config:?} current_file={current:?} paths_source={:?} microphone={mic:?}",
            s.installed,
            s.loaded,
            env!("CARGO_PKG_VERSION"),
            s.paths.source
        )?;
    } else {
        let saved = if s.paths.source == "saved" {
            " (saved)"
        } else {
            ""
        };
        let rows = vec![
            ("Installed".into(), s.installed.to_string()),
            ("Loaded by macOS".into(), s.loaded.to_string()),
            ("CLI version".into(), env!("CARGO_PKG_VERSION").into()),
            ("Running service version".into(), version),
            ("Service file".into(), plist),
            (format!("Recording directory{saved}"), output),
            (format!("Configuration file{saved}"), config),
            ("Current recording file".into(), current),
            ("Current microphone".into(), mic),
        ];
        let mut b = Builder::default();
        for (a, bv) in rows {
            b.push_record([a, bv]);
        }
        let mut table = b.build();
        table.with(Style::empty()).with(Padding::new(0, 2, 0, 0));
        for line in table.to_string().lines() {
            writeln!(out, "{}", line.trim_end())?;
        }
    }
    if details && s.loaded {
        let mut remaining = s.diagnostics.as_slice();
        while !remaining.is_empty() {
            match std::str::from_utf8(remaining) {
                Ok(text) => {
                    out.write_all(fold(text, home).as_bytes())?;
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    out.write_all(
                        fold(
                            std::str::from_utf8(&remaining[..valid])
                                .expect("validated UTF-8 prefix"),
                            home,
                        )
                        .as_bytes(),
                    )?;
                    let end = valid + error.error_len().unwrap_or(remaining.len() - valid);
                    out.write_all(&remaining[valid..end])?;
                    remaining = &remaining[end..];
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/unit/output.rs"]
mod tests;
