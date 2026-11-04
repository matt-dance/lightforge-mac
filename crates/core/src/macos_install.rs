//! Make a macOS WoW install launchable after TACT extraction.
//!
//! The install manifest has no Unix mode bits, so extracted Mach-O binaries
//! land `0644`. Gatekeeper also stamps `com.apple.quarantine` on files that
//! arrived over the network. TACT lists CEF as a *shallow* framework
//! (`Resources/`, `Libraries/` at the top), while dyld and the in-game
//! Battle.net browser expect the versioned layout (`Versions/A` + `Current`).
//!
//! This module:
//!
//! 1. Rebuilds `.framework` trees in place (no re-download).
//! 2. Sets `+x` on Mach-O and `#!` helpers.
//! 3. Clears quarantine.
//! 4. Ad-hoc signs **unsigned** `.app` / `.framework` bundles under
//!    `Utils/` (BlizzardBrowser, its CEF helpers, Blizzard Error) so the
//!    in-game login window can launch. Nothing inside
//!    `World of Warcraft.app` is re-signed: that bundle ships with
//!    Blizzard's Developer ID (`Team G847MC6JZ5`). Ad-hoc signing a nested
//!    helper such as `WowVoiceProxy.app` invalidates the parent's sealed
//!    resources; `--force` then strips the Developer ID and the client
//!    aborts ~13s later (ERROR #132). `icudt*.dat` in `Contents/MacOS` is
//!    expected Blizzard data, not a reason to re-sign or warn.
//!
//! `execute` runs this after every install-file extract: a fresh install
//! *and* later patches that replace `World of Warcraft.app` /
//! `BlizzardBrowser.app`. The CASC store under `Data/` is skipped: a
//! million archived blobs are not executables, and an 85 GB repair must
//! not walk them.

use std::fs::File;
use std::io::Read;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Directories that never hold a client binary or a CEF framework.
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
    pub assembled_frameworks: usize,
    pub signed: usize,
    pub cleared_quarantine: bool,
    pub codesign_ok: usize,
    pub codesign_failed: usize,
    /// Install-manifest files rewritten by a CDN repair. Zero for a local pass.
    pub extracted: usize,
    pub extract_failed: usize,
}

impl std::fmt::Display for Report {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.extracted > 0 || self.extract_failed > 0 {
            write!(f, "re-extracted {} install files", self.extracted)?;
            if self.extract_failed > 0 {
                write!(f, " ({} failed)", self.extract_failed)?;
            }
            write!(f, ", ")?;
        }
        write!(f, "set +x on {} files", self.made_executable)?;
        if self.already_executable > 0 {
            write!(f, " ({} already executable)", self.already_executable)?;
        }
        if self.assembled_frameworks > 0 {
            write!(f, ", assembled {} framework(s)", self.assembled_frameworks)?;
        }
        if self.signed > 0 {
            write!(f, ", ad-hoc signed {}", self.signed)?;
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
        0xFEED_FACE
            | 0xFEED_FACF
            | 0xCEFA_EDFE
            | 0xCFFA_EDFE
            | 0xCAFE_BABE
            | 0xBEBA_FECA
            | 0xCAFE_BABF
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

/// Whether the product's launch binary is present and lacks `+x`, or a CEF
/// framework is still in the shallow TACT layout.
pub fn needs_fixup(install: &Path, code: &crate::product::ProductCode) -> bool {
    let local = crate::state::Local::inspect_at(install, code);
    let Some(exe) = local.executable else {
        return false;
    };
    let binary = crate::paths::launch_binary(&exe);
    let mode_wrong = match std::fs::metadata(&binary) {
        Ok(meta) if meta.is_file() => meta.permissions().mode() & 0o111 == 0,
        Ok(_) => false,
        Err(_) => true,
    };
    mode_wrong || frameworks_need_assembly(install)
}

/// Whether `World of Warcraft.app` must be re-extracted from the CDN.
///
/// True when a previous fixup ad-hoc signed a nested helper (or `--force`
/// signed the parent and failed), leaving the bundle unsigned or with an
/// ad-hoc helper inside a Developer ID parent. Local `+x` / CEF assembly
/// cannot restore Blizzard's signature; [`crate::tact::install::repair_install_files`]
/// can.
pub fn needs_identity_repair(install: &Path, code: &crate::product::ProductCode) -> bool {
    let local = crate::state::Local::inspect_at(install, code);
    let Some(exe) = local.executable else {
        return false;
    };
    if !crate::paths::is_app_bundle(&exe) {
        return false;
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = exe;
        false
    }
    #[cfg(target_os = "macos")]
    {
        game_app_needs_cdn_repair(has_code_signature(&exe), nested_helpers_are_adhoc(&exe))
    }
}

/// Decision behind [`needs_identity_repair`], kept pure so Linux CI covers it.
pub fn game_app_needs_cdn_repair(parent_has_signature: bool, nested_helper_adhoc: bool) -> bool {
    !parent_has_signature || nested_helper_adhoc
}

/// Whether `codesign --verify` stderr is only Blizzard's unsigned ICU data
/// file, not a broken bundle signature.
///
/// Official `World of Warcraft.app` ships `Contents/MacOS/icudt*.dat` next
/// to the Mach-O. `codesign --verify` reports that subcomponent as unsigned
/// even without `--deep`. The outer bundle still has a valid Developer ID
/// (`com.blizzard.worldofwarcraft`) and launches.
pub fn verify_stderr_is_expected_icu_data(stderr: &str) -> bool {
    let text = stderr.trim();
    if text.is_empty() || !text.contains("In subcomponent:") {
        return false;
    }
    if text.contains("a sealed resource is missing or invalid") {
        return false;
    }
    let mut saw_sub = false;
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("In subcomponent:") else {
            continue;
        };
        saw_sub = true;
        let path = rest.trim();
        let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
        let name = name.to_ascii_lowercase();
        if !name.starts_with("icudt") || !name.ends_with(".dat") {
            return false;
        }
    }
    saw_sub
}

/// Whether `path` is `World of Warcraft.app` or a file inside it.
///
/// Nested helpers (`Contents/Helpers/WowVoiceProxy.app`) share the parent's
/// Developer ID. Ad-hoc signing them invalidates the sealed resources and
/// is what then tempts `--force` onto the parent.
pub fn path_is_inside_game_app(path: &Path) -> bool {
    path.components().any(|component| {
        component
            .as_os_str()
            .to_str()
            .is_some_and(|name| name.eq_ignore_ascii_case("World of Warcraft.app"))
    })
}

/// Whether `codesign -dv` stderr describes an ad-hoc signature.
pub fn codesign_display_is_adhoc(stderr: &str) -> bool {
    stderr.lines().any(|line| {
        let lower = line.trim().to_ascii_lowercase();
        lower == "signature=adhoc" || (lower.contains("flags=") && lower.contains("adhoc"))
    })
}

/// Whether a bundle should be ad-hoc signed.
///
/// Skip when `path` is inside `World of Warcraft.app` (Blizzard's Developer
/// ID, including nested helpers). Skip when `codesign --verify` succeeded,
/// or when it only complained about `icudt*.dat` on a bundle that already
/// has an identity. Sign when the signature is missing or broken (CEF /
/// BlizzardBrowser after TACT).
pub fn bundle_needs_adhoc_sign(
    verify_ok: bool,
    has_code_signature: bool,
    verify_stderr: &str,
    inside_game_app: bool,
) -> bool {
    if inside_game_app {
        return false;
    }
    if verify_ok {
        return false;
    }
    if has_code_signature && verify_stderr_is_expected_icu_data(verify_stderr) {
        return false;
    }
    true
}

/// Walk `root`, rebuild shallow `.framework` trees, set `+x` on Mach-O /
/// scripts, and on macOS clear quarantine and ad-hoc sign each bundle.
pub fn prepare_tree(root: &Path) -> Report {
    let mut report = Report::default();
    assemble_frameworks(root, &mut report);
    walk(root, &mut report);
    #[cfg(target_os = "macos")]
    {
        report.cleared_quarantine = clear_quarantine(root);
        adhoc_sign(root, &mut report);
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

/// TACT lists CEF as `framework/Resources` and `framework/Libraries` with no
/// `Versions/A`. Official CEF (and Xcode 26 dyld) wants:
///
/// ```text
/// Chromium Embedded Framework.framework/
///   Chromium Embedded Framework -> Versions/Current/…
///   Libraries -> Versions/Current/Libraries
///   Resources -> Versions/Current/Resources
///   Versions/
///     A/   (the real files)
///     Current -> A
/// ```
///
/// Moving the extracted files into `Versions/A` and putting relative
/// symlinks on top is idempotent, and a later extract writes through those
/// links.
fn assemble_frameworks(root: &Path, report: &mut Report) {
    let mut frameworks = Vec::new();
    collect_frameworks(root, &mut frameworks);
    for framework in frameworks {
        match assemble_one(&framework) {
            Ok(true) => {
                report.assembled_frameworks += 1;
                tracing::info!("assembled framework {}", framework.display());
            }
            Ok(false) => {}
            Err(e) => tracing::warn!("could not assemble {}: {e}", framework.display()),
        }
    }
}

fn collect_frameworks(dir: &Path, out: &mut Vec<PathBuf>) {
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
        if !file_type.is_dir() {
            continue;
        }
        if SKIP.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
            continue;
        }
        if is_framework(&path) {
            out.push(path);
            continue;
        }
        collect_frameworks(&path, out);
    }
}

fn is_framework(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("framework"))
        && path.is_dir()
}

fn assemble_one(framework: &Path) -> std::io::Result<bool> {
    let versions = framework.join("Versions");
    let version_a = versions.join("A");
    if version_a.is_dir() {
        return ensure_framework_symlinks(framework);
    }

    let mut move_names = Vec::new();
    let Ok(entries) = std::fs::read_dir(framework) else {
        return Ok(false);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        if name == "Versions" {
            continue;
        }
        move_names.push(name);
    }
    if move_names.is_empty() {
        return Ok(false);
    }

    // A framework we did not extract (empty shell) is not worth wrapping.
    let has_payload = move_names.iter().any(|name| {
        let n = name.to_string_lossy();
        n.eq_ignore_ascii_case("Resources")
            || n.eq_ignore_ascii_case("Libraries")
            || n.eq_ignore_ascii_case("_CodeSignature")
            || !n.starts_with('.')
    });
    if !has_payload {
        return Ok(false);
    }

    std::fs::create_dir_all(&version_a)?;
    for name in &move_names {
        let from = framework.join(name);
        let to = version_a.join(name);
        if to.exists() {
            continue;
        }
        std::fs::rename(&from, &to)?;
    }
    ensure_framework_symlinks(framework)?;
    Ok(true)
}

fn ensure_framework_symlinks(framework: &Path) -> std::io::Result<bool> {
    let versions = framework.join("Versions");
    let version_a = versions.join("A");
    if !version_a.is_dir() {
        return Ok(false);
    }
    replace_symlink(&versions.join("Current"), Path::new("A"))?;

    let mut changed = false;
    let Ok(entries) = std::fs::read_dir(&version_a) else {
        return Ok(false);
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let link = framework.join(&name);
        let target = Path::new("Versions").join("Current").join(&name);
        if replace_symlink(&link, &target)? {
            changed = true;
        }
    }
    Ok(changed)
}

/// Create or replace `path` as a symlink to `target`. Returns whether the
/// link was created or retargeted.
fn replace_symlink(path: &Path, target: &Path) -> std::io::Result<bool> {
    if let Ok(meta) = path.symlink_metadata() {
        if meta.file_type().is_symlink() {
            if std::fs::read_link(path).ok().as_deref() == Some(target) {
                return Ok(false);
            }
            std::fs::remove_file(path)?;
        } else if meta.is_dir() {
            std::fs::remove_dir_all(path)?;
        } else {
            std::fs::remove_file(path)?;
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::os::unix::fs::symlink(target, path)?;
    Ok(true)
}

fn frameworks_need_assembly(root: &Path) -> bool {
    let mut frameworks = Vec::new();
    collect_frameworks(root, &mut frameworks);
    frameworks.iter().any(|fw| {
        let current = fw.join("Versions").join("Current");
        if current.is_symlink() || current.is_dir() {
            return false;
        }
        fw.join("Resources").is_dir() || fw.join("Libraries").is_dir()
    })
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

/// Ad-hoc sign `.app` and `.framework` bundles whose signature is missing
/// or broken. `World of Warcraft.app` and everything inside it is left
/// alone: a valid Developer ID must not be replaced, and signing a nested
/// helper invalidates the parent's sealed resources. `--deep` is not used:
/// it treats data files next to the Mach-O as unsigned nested code.
#[cfg(target_os = "macos")]
fn adhoc_sign(root: &Path, report: &mut Report) {
    let mut bundles = Vec::new();
    collect_signable(root, &mut bundles);
    bundles.sort_by_key(|p| std::cmp::Reverse(p.components().count()));
    for bundle in bundles {
        let inside_game = path_is_inside_game_app(&bundle);
        let (verify_ok, stderr) = codesign_verify(&bundle);
        let has_sig = has_code_signature(&bundle);
        if !bundle_needs_adhoc_sign(verify_ok, has_sig, &stderr, inside_game) {
            tracing::debug!("leaving existing signature on {}", bundle.display());
            continue;
        }
        if sign_one(&bundle) {
            report.signed += 1;
            tracing::info!("ad-hoc signed {}", bundle.display());
        } else {
            tracing::warn!("ad-hoc sign failed for {}", bundle.display());
        }
    }
}

#[cfg(target_os = "macos")]
fn collect_signable(dir: &Path, out: &mut Vec<PathBuf>) {
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
        if !file_type.is_dir() {
            continue;
        }
        if SKIP.iter().any(|skip| name.eq_ignore_ascii_case(skip)) {
            continue;
        }
        if crate::paths::is_app_bundle(&path) || is_framework(&path) {
            out.push(path.clone());
            // Nested helpers inside the game .app stay Blizzard-signed.
            if crate::paths::is_app_bundle(&path) && path_is_inside_game_app(&path) {
                continue;
            }
        }
        collect_signable(&path, out);
    }
}

#[cfg(target_os = "macos")]
fn sign_one(path: &Path) -> bool {
    let status = std::process::Command::new("codesign")
        .args(["--force", "--sign", "-"])
        .arg(path)
        .status();
    matches!(status, Ok(status) if status.success())
}

#[cfg(target_os = "macos")]
fn codesign_verify(path: &Path) -> (bool, String) {
    match std::process::Command::new("codesign")
        .args(["--verify", "--verbose=2"])
        .arg(path)
        .output()
    {
        Ok(output) => (
            output.status.success(),
            String::from_utf8_lossy(&output.stderr).into_owned(),
        ),
        Err(e) => (false, e.to_string()),
    }
}

#[cfg(target_os = "macos")]
fn has_code_signature(path: &Path) -> bool {
    std::process::Command::new("codesign")
        .args(["-d"])
        .arg(path)
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

#[cfg(target_os = "macos")]
fn nested_helpers_are_adhoc(app: &Path) -> bool {
    let helpers = app.join("Contents").join("Helpers");
    let Ok(entries) = std::fs::read_dir(helpers) else {
        return false;
    };
    entries.flatten().any(|entry| {
        let path = entry.path();
        crate::paths::is_app_bundle(&path) && bundle_is_adhoc(&path)
    })
}

#[cfg(target_os = "macos")]
fn bundle_is_adhoc(path: &Path) -> bool {
    match std::process::Command::new("codesign")
        .args(["-d", "-v"])
        .arg(path)
        .output()
    {
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            output.status.success() && codesign_display_is_adhoc(&stderr)
        }
        Err(_) => false,
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
    // No `--deep`: `Contents/MacOS/icudt78l.dat` is a data file the official
    // bundle also ships, and `--deep` reports it as unsigned nested code.
    let (ok, stderr) = codesign_verify(app);
    if ok {
        report.codesign_ok += 1;
        tracing::info!("codesign --verify ok: {}", app.display());
        return;
    }
    if has_code_signature(app) && verify_stderr_is_expected_icu_data(&stderr) {
        report.codesign_ok += 1;
        tracing::debug!(
            "codesign --verify notes unsigned icudt data in {}; expected for Blizzard's bundle",
            app.display()
        );
        return;
    }
    report.codesign_failed += 1;
    if path_is_inside_game_app(app) {
        tracing::warn!(
            "codesign --verify failed for {} (re-extract the app from the CDN; do not ad-hoc sign it): {}",
            app.display(),
            stderr.trim()
        );
        return;
    }
    tracing::warn!(
        "codesign --verify failed for {}: {}",
        app.display(),
        stderr.trim()
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::os::unix::fs::symlink;

    #[test]
    fn blizzard_icu_dat_is_expected_verify_noise() {
        let stderr = "\
World of Warcraft.app: code object is not signed at all
In subcomponent: /Games/_forever_/World of Warcraft.app/Contents/MacOS/icudt78l.dat
";
        assert!(verify_stderr_is_expected_icu_data(stderr));
        assert!(!bundle_needs_adhoc_sign(false, true, stderr, false));
    }

    #[test]
    fn an_unsigned_bundle_is_signed() {
        assert!(!verify_stderr_is_expected_icu_data(
            "World of Warcraft.app: code object is not signed at all"
        ));
        assert!(bundle_needs_adhoc_sign(
            false,
            false,
            "code object is not signed at all",
            false
        ));
    }

    #[test]
    fn a_valid_verify_is_not_re_signed() {
        assert!(!bundle_needs_adhoc_sign(true, true, "", false));
    }

    #[test]
    fn a_sealed_resource_missing_is_still_signed() {
        let stderr = "\
a sealed resource is missing or invalid
In subcomponent: /Utils/BlizzardBrowser.app/Contents/Frameworks/Chromium Embedded Framework.framework
";
        assert!(!verify_stderr_is_expected_icu_data(stderr));
        assert!(bundle_needs_adhoc_sign(false, true, stderr, false));
    }

    #[test]
    fn another_unsigned_subcomponent_is_not_icu_noise() {
        let stderr = "\
code object is not signed at all
In subcomponent: /World of Warcraft.app/Contents/MacOS/helper
";
        assert!(!verify_stderr_is_expected_icu_data(stderr));
        assert!(bundle_needs_adhoc_sign(false, true, stderr, false));
    }

    #[test]
    fn nothing_inside_the_game_app_is_ad_hoc_signed() {
        let sealed = "\
a sealed resource is missing or invalid
In subcomponent: /World of Warcraft.app/Contents/Helpers/WowVoiceProxy.app
";
        assert!(!bundle_needs_adhoc_sign(false, true, sealed, true));
        assert!(!bundle_needs_adhoc_sign(
            false,
            false,
            "code object is not signed at all",
            true
        ));
        assert!(!bundle_needs_adhoc_sign(true, true, "", true));
    }

    #[test]
    fn game_app_paths_cover_the_bundle_and_nested_helpers() {
        assert!(path_is_inside_game_app(Path::new(
            "/Games/_forever_/World of Warcraft.app"
        )));
        assert!(path_is_inside_game_app(Path::new(
            "/Games/_forever_/World of Warcraft.app/Contents/Helpers/WowVoiceProxy.app"
        )));
        assert!(path_is_inside_game_app(Path::new(
            "/Games/_forever_/WORLD OF WARCRAFT.APP/Contents/Info.plist"
        )));
        assert!(!path_is_inside_game_app(Path::new(
            "/Games/_forever_/Utils/BlizzardBrowser.app"
        )));
        assert!(!path_is_inside_game_app(Path::new(
            "/Games/_forever_/Utils/BlizzardBrowser.app/Contents/Frameworks/BlizzardBrowser Helper.app"
        )));
        assert!(!path_is_inside_game_app(Path::new(
            "/Games/_forever_/Utils/Blizzard Error.app"
        )));
    }

    #[test]
    fn adhoc_display_text_is_recognised() {
        assert!(codesign_display_is_adhoc(
            "Executable=/tmp/WowVoiceProxy.app/Contents/MacOS/WowVoiceProxy\n\
             Identifier=com.blizzard.wowvoiceproxy\n\
             Format=app bundle with Mach-O thin (arm64)\n\
             CodeDirectory v=20400 size=300 flags=0x2(adhoc) hashes=8+2\n\
             Signature=adhoc\n"
        ));
        assert!(codesign_display_is_adhoc(
            "CodeDirectory v=20500 size=400 flags=0x10002(runtime,adhoc) hashes=12+2"
        ));
        assert!(!codesign_display_is_adhoc(
            "Authority=Developer ID Application: Blizzard Entertainment, Inc. (G847MC6JZ5)\n\
             TeamIdentifier=G847MC6JZ5\n"
        ));
        assert!(!codesign_display_is_adhoc(""));
    }

    #[test]
    fn a_stripped_or_nested_adhoc_game_app_needs_cdn_repair() {
        assert!(game_app_needs_cdn_repair(false, false));
        assert!(game_app_needs_cdn_repair(true, true));
        assert!(game_app_needs_cdn_repair(false, true));
        assert!(!game_app_needs_cdn_repair(true, false));
    }

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
        file.write_all(&[0xCF, 0xFA, 0xED, 0xFE, 0, 0, 0, 0])
            .unwrap();
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

    fn shallow_cef(root: &Path) -> PathBuf {
        let fw = root
            .join("Utils")
            .join("BlizzardBrowser.app")
            .join("Contents")
            .join("Frameworks")
            .join("Chromium Embedded Framework.framework");
        let resources = fw.join("Resources");
        let libraries = fw.join("Libraries");
        std::fs::create_dir_all(&resources).unwrap();
        std::fs::create_dir_all(&libraries).unwrap();
        std::fs::write(resources.join("Info.plist"), b"<plist/>").unwrap();
        std::fs::write(libraries.join("vk_swiftshader_icd.json"), b"{}").unwrap();
        std::fs::write(
            fw.join("Chromium Embedded Framework"),
            [0xCF, 0xFA, 0xED, 0xFE],
        )
        .unwrap();
        fw
    }

    #[test]
    fn a_shallow_cef_framework_becomes_versioned() {
        let root = tempfile::tempdir().unwrap();
        let fw = shallow_cef(root.path());

        let report = prepare_tree(root.path());
        assert_eq!(report.assembled_frameworks, 1, "{report:?}");

        let current = fw.join("Versions").join("Current");
        assert!(current.is_symlink(), "Versions/Current must be a symlink");
        assert_eq!(std::fs::read_link(&current).unwrap(), Path::new("A"));

        let info = fw.join("Resources").join("Info.plist");
        assert!(info.is_file(), "Resources convenience link must resolve");
        assert_eq!(std::fs::read(&info).unwrap(), b"<plist/>");
        assert!(
            fw.join("Resources")
                .symlink_metadata()
                .unwrap()
                .file_type()
                .is_symlink(),
            "top-level Resources is a symlink, not a second copy"
        );

        let binary = fw.join("Chromium Embedded Framework");
        assert!(binary.is_file());
        assert!(binary.symlink_metadata().unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read(&binary).unwrap(), [0xCF, 0xFA, 0xED, 0xFE]);

        // A second pass is a no-op: the files stay where they are.
        let again = prepare_tree(root.path());
        assert_eq!(again.assembled_frameworks, 0, "{again:?}");
        assert_eq!(std::fs::read(&info).unwrap(), b"<plist/>");
    }

    #[test]
    fn extract_after_assembly_writes_through_the_symlinks() {
        let root = tempfile::tempdir().unwrap();
        let fw = shallow_cef(root.path());
        prepare_tree(root.path());

        // A later extract resolves the same POSIX path TACT listed; the
        // convenience symlink must carry the bytes into Versions/A.
        let dest = fw.join("Resources").join("Info.plist");
        std::fs::write(&dest, b"<plist updated/>").unwrap();
        assert_eq!(
            std::fs::read(
                fw.join("Versions")
                    .join("A")
                    .join("Resources")
                    .join("Info.plist")
            )
            .unwrap(),
            b"<plist updated/>"
        );
    }

    #[test]
    fn an_already_versioned_framework_is_left_with_its_links() {
        let root = tempfile::tempdir().unwrap();
        let fw = root.path().join("Foo.framework");
        let a = fw.join("Versions").join("A");
        std::fs::create_dir_all(a.join("Resources")).unwrap();
        std::fs::write(a.join("Resources").join("Info.plist"), b"ok").unwrap();
        symlink("A", fw.join("Versions").join("Current")).unwrap();
        symlink("Versions/Current/Resources", fw.join("Resources")).unwrap();

        let report = prepare_tree(root.path());
        assert_eq!(report.assembled_frameworks, 0, "{report:?}");
        assert_eq!(
            std::fs::read(fw.join("Resources").join("Info.plist")).unwrap(),
            b"ok"
        );
    }

    #[test]
    fn a_patch_that_replaces_the_browser_is_assembled_again() {
        let root = tempfile::tempdir().unwrap();
        let fw = shallow_cef(root.path());
        let first = prepare_tree(root.path());
        assert_eq!(first.assembled_frameworks, 1, "{first:?}");
        assert!(fw.join("Versions").join("Current").is_symlink());

        // A TACT update re-extracts the install-manifest paths as regular
        // files, replacing the assembled .app / .framework.
        std::fs::remove_dir_all(&fw).unwrap();
        let fw = shallow_cef(root.path());
        assert!(
            !fw.join("Versions").exists(),
            "the patch left the shallow TACT layout"
        );

        let patched = prepare_tree(root.path());
        assert_eq!(patched.assembled_frameworks, 1, "{patched:?}");
        assert!(fw.join("Versions").join("Current").is_symlink());
        assert_eq!(
            std::fs::read(fw.join("Resources").join("Info.plist")).unwrap(),
            b"<plist/>"
        );
        let mode = std::fs::metadata(fw.join("Chromium Embedded Framework"))
            .unwrap()
            .permissions()
            .mode()
            & 0o111;
        assert_ne!(mode, 0, "the replaced Mach-O must be executable again");
    }

    #[test]
    fn needs_fixup_sees_a_shallow_framework() {
        let root = tempfile::tempdir().unwrap();
        let flavor = root.path().join("_forever_");
        let macos = flavor
            .join("World of Warcraft.app")
            .join("Contents")
            .join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let bin = macos.join("World of Warcraft");
        std::fs::write(&bin, [0xCF, 0xFA, 0xED, 0xFE]).unwrap();
        std::fs::set_permissions(&bin, PermissionsExt::from_mode(0o755)).unwrap();
        shallow_cef(&flavor);

        // inspect_at needs .build.info + store files to count as installed,
        // but needs_fixup only looks at the launch binary and frameworks.
        let code = crate::product::ProductCode::from("wowf".to_owned());
        assert!(needs_fixup(root.path(), &code) || frameworks_need_assembly(root.path()));
        prepare_tree(root.path());
        assert!(!frameworks_need_assembly(root.path()));
    }
}
