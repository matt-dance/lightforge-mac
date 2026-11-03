//! Make a macOS WoW install launchable after TACT extraction.
//!
//! The install manifest has no Unix mode bits, so extracted Mach-O binaries
//! land `0644`. Gatekeeper also stamps `com.apple.quarantine` on files that
//! arrived over the network. This module sets `+x` on Mach-O and `#!`
//! helpers, clears quarantine, and checks each `.app` with `codesign`.
//!
//! The CASC store under `Data/` is skipped: a million archived blobs are not
//! executables.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// Directories that never hold a client binary.
const SKIP: &[&str] = &[
    "Data",
    "Cache",
    "WTF",
    "Logs",
    "Interface",
    "Errors",
    "Screenshots",
    "Fonts",
];

/// What a pass over an install changed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub made_executable: usize,
    pub already_executable: usize,
    pub cleared_quarantine: bool,
    pub codesign_ok: usize,
    pub codesign_failed: usize,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "set +x on {} files", self.made_executable)?;
        if self.already_executable > 0 {
            write!(f, " ({} already executable)", self.already_executable)?;
        }
        if self.cleared_quarantine {
            write!(f, ", cleared quarantine")?;
        }
        if self.codesign_ok + self.codesign_failed > 0 {
            write!(
                f,
                ", codesign {} ok / {} failed",
                self.codesign_ok, self.codesign_failed
            )?;
        }
        Ok(())
    }
}

/// Mach-O 32/64, byte-swapped, and fat/universal magics.
pub fn looks_like_mach_o(bytes: &[u8]) -> bool {
    if bytes.len() < 4 {
        return false;
    }
    let magic = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    matches!(
        magic,
        0xFEED_FACE | 0xFEED_FACF | 0xCEFA_EDFE | 0xCFFA_EDFE | 0xCAFE_BABE | 0xBEBA_FECA | 0xCAFE_BABF
    )
}

fn looks_like_script(bytes: &[u8]) -> bool {
    bytes.starts_with(b"#!")
}

/// Mode an extracted install-manifest file should be written with.
pub fn install_file_mode(bytes: &[u8]) -> u32 {
    if looks_like_mach_o(bytes) || looks_like_script(bytes) {
        0o755
    } else {
        0o644
    }
}

/// Whether the product's launch binary is present and lacks `+x`.
pub fn needs_fixup(install: &Path, code: &crate::product::ProductCode) -> bool {
    let local = crate::state::Local::inspect_at(install, code);
    let Some(exe) = local.executable else {
        return false;
    };
    let binary = crate::paths::launch_binary(&exe);
    match std::fs::metadata(&binary) {
        Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o111 == 0,
        Ok(_) => false,
        Err(_) => true,
    }
}

/// Walk `root`, set `+x` on Mach-O / scripts, and on macOS clear quarantine
/// and verify each `.app` bundle's signature.
pub fn prepare_tree(root: &Path) -> Report {
    let mut report = Report::default();
    walk(root, &mut report);
    #[cfg(target_os = "macos")]
    {
        report.cleared_quarantine = clear_quarantine(root);
        verify_apps(root, &mut report);
    }
    report
}

/// The same pass over a whole WoW install root, skipping the CASC store.
pub fn prepare_install(install: &Path) -> Report {
    prepare_tree(install)
}

fn walk(dir: &Path, report: &mut Report) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            if SKIP.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
                continue;
            }
            walk(&path, report);
            continue;
        }
        if file_type.is_file() {
            ensure_executable(&path, report);
        }
    }
}

fn ensure_executable(path: &Path, report: &mut Report) {
    let Ok(meta) = std::fs::metadata(path) else {
        return;
    };
    if !meta.is_file() {
        return;
    }
    let mode = meta.permissions().mode();
    if mode & 0o111 != 0 {
        if peek_executable(path) {
            report.already_executable += 1;
        }
        return;
    }
    if !peek_executable(path) {
        return;
    }
    let mut perms = meta.permissions();
    perms.set_mode(mode | 0o111);
    if std::fs::set_permissions(path, perms).is_ok() {
        report.made_executable += 1;
        tracing::info!("set +x on {}", path.display());
    }
}

fn peek_executable(path: &Path) -> bool {
    let mut buf = [0u8; 4];
    let Ok(mut file) = File::open(path) else {
        return false;
    };
    let Ok(n) = file.read(&mut buf) else {
        return false;
    };
    looks_like_mach_o(&buf[..n]) || looks_like_script(&buf[..n])
}

#[cfg(target_os = "macos")]
fn clear_quarantine(root: &Path) -> bool {
    let status = std::process::Command::new("xattr")
        .args(["-dr", "com.apple.quarantine"])
        .arg(root)
        .status();
    match status {
        Ok(status) if status.success() => {
            tracing::info!("cleared quarantine on {}", root.display());
            true
        }
        Ok(status) => {
            tracing::warn!("xattr -dr quarantine on {} exited {status}", root.display());
            false
        }
        Err(e) => {
            tracing::warn!("xattr -dr quarantine on {}: {e}", root.display());
            false
        }
    }
}

#[cfg(target_os = "macos")]
fn verify_apps(dir: &Path, report: &mut Report) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_dir() {
            continue;
        }
        if crate::paths::is_app_bundle(&path) {
            verify_one_app(&path, report);
        } else if SKIP.iter().any(|skip| {
            entry
                .file_name()
                .to_string_lossy()
                .eq_ignore_ascii_case(skip)
        }) {
            continue;
        } else {
            verify_apps(&path, report);
        }
    }
}

#[cfg(target_os = "macos")]
fn verify_one_app(app: &Path, report: &mut Report) {
    let output = std::process::Command::new("codesign")
        .args(["--verify", "--deep", "--verbose=2"])
        .arg(app)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            report.codesign_ok += 1;
            tracing::info!("codesign --verify --deep ok: {}", app.display());
        }
        Ok(output) => {
            report.codesign_failed += 1;
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                "codesign --verify --deep failed for {}: {}",
                app.display(),
                stderr.trim()
            );
        }
        Err(e) => {
            report.codesign_failed += 1;
            tracing::warn!("codesign --verify --deep {}: {e}", app.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn mach_o_magics_are_recognised() {
        assert!(looks_like_mach_o(&[0xCF, 0xFA, 0xED, 0xFE]));
        assert!(looks_like_mach_o(&[0xFE, 0xED, 0xFA, 0xCF]));
        assert!(looks_like_mach_o(&[0xCA, 0xFE, 0xBA, 0xBE]));
        assert!(!looks_like_mach_o(b"MZ\x90\x00"));
        assert!(!looks_like_mach_o(b"PK\x03\x04"));
        assert!(!looks_like_mach_o(&[]));
    }

    #[test]
    fn scripts_and_mach_o_are_written_executable() {
        assert_eq!(install_file_mode(b"\xcf\xfa\xed\xfe...."), 0o755);
        assert_eq!(install_file_mode(b"#!/bin/sh\necho hi\n"), 0o755);
        assert_eq!(install_file_mode(b"MZ...."), 0o644);
        assert_eq!(install_file_mode(b"plain text"), 0o644);
    }

    #[test]
    fn a_tree_walk_sets_plus_x_and_skips_the_store() {
        let root = tempfile::tempdir().unwrap();
        let flavor = root.path().join("_forever_");
        let macos = flavor
            .join("World of Warcraft.app")
            .join("Contents")
            .join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let bin = macos.join("World of Warcraft");
        let mut file = File::create(&bin).unwrap();
        file.write_all(&[0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0]).unwrap();
        drop(file);
        std::fs::set_permissions(&bin, PermissionsExt::from_mode(0o644)).unwrap();

        let store = root.path().join("Data").join("data");
        std::fs::create_dir_all(&store).unwrap();
        let blob = store.join("not-a-binary");
        std::fs::write(&blob, [0xCF, 0xFA, 0xED, 0xFE]).unwrap();
        std::fs::set_permissions(&blob, PermissionsExt::from_mode(0o644)).unwrap();

        let report = prepare_tree(root.path());
        assert_eq!(report.made_executable, 1, "{report:?}");
        let mode = std::fs::metadata(&bin).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o755, "{mode:o}");
        let store_mode = std::fs::metadata(&blob).unwrap().permissions().mode() & 0o777;
        assert_eq!(store_mode, 0o644, "Data/ must not be walked");
    }

    #[test]
    fn a_file_that_already_has_plus_x_is_left_alone() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("helper");
        std::fs::write(&bin, [0xCF, 0xFA, 0xED, 0xFE]).unwrap();
        std::fs::set_permissions(&bin, PermissionsExt::from_mode(0o755)).unwrap();
        let report = prepare_tree(root.path());
        assert_eq!(report.made_executable, 0);
        assert_eq!(report.already_executable, 1);
    }
}
