//! Bake the GitHub release tag into the binary so the update check can
//! see `-mac.N`. `CARGO_PKG_VERSION` stays the upstream SemVer in
//! Cargo.toml; a tagged macOS release is `v<upstream>-mac.N`.

fn main() {
    println!("cargo:rerun-if-env-changed=LIGHTFORGE_VERSION");
    println!("cargo:rerun-if-env-changed=GITHUB_REF_NAME");

    let version = explicit_version()
        .or_else(version_from_github_tag)
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").expect("CARGO_PKG_VERSION"));
    println!("cargo:rustc-env=LIGHTFORGE_VERSION={version}");
}

fn explicit_version() -> Option<String> {
    std::env::var("LIGHTFORGE_VERSION")
        .ok()
        .filter(|s| !s.is_empty())
}

/// `GITHUB_REF_NAME` is `v1.0.0-beta.9-mac.3` on a tag workflow. Branch
/// names are not versions, so only a `*-mac.N` tag is taken.
fn version_from_github_tag() -> Option<String> {
    let tag = std::env::var("GITHUB_REF_NAME").ok()?;
    let bare = tag.trim().trim_start_matches('v');
    let (_, n) = bare.rsplit_once("-mac.")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if !bare.starts_with(|c: char| c.is_ascii_digit()) {
        return None;
    }
    Some(bare.to_owned())
}
