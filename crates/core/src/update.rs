//! Is there a newer Lightforge?
//!
//! Asked once per start; installing is [`crate::upgrade`]'s. The full release
//! list, not `releases/latest` (which skips prereleases), compared by
//! [`semver`] precedence (`1.0.0-beta.10` > `1.0.0-beta.2`) and then by this
//! fork's `-mac.N` suffix (`1.0.0-beta.9-mac.3` > `1.0.0-beta.9-mac.2`).
//!
//! This is the macOS fork: releases live on `matt-dance/lightforge-mac`,
//! tagged `v<upstream>-mac.N`.

use serde::{Deserialize, Serialize};

use crate::net::{ETagCache, Http, NetError};

/// GitHub owner/repo this build checks and links to.
pub const GITHUB_REPO: &str = "matt-dance/lightforge-mac";

/// GitHub Releases endpoint for this fork.
const RELEASES_URL: &str = "https://api.github.com/repos/matt-dance/lightforge-mac/releases";

/// Where to send someone who wants the new build.
pub const RELEASES_PAGE: &str = "https://github.com/matt-dance/lightforge-mac/releases";

/// Version this binary reports, including `-mac.N` on a tagged macOS
/// release. `CARGO_PKG_VERSION` is the upstream SemVer in Cargo.toml;
/// `build.rs` sets this from `LIGHTFORGE_VERSION`, or from
/// `GITHUB_REF_NAME` on a `v*-mac.N` tag.
pub const VERSION: &str = env!("LIGHTFORGE_VERSION");

/// One published release, as much of it as this needs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Release {
    /// Tag as published, e.g. `v1.0.0-beta.9-mac.3`.
    pub tag_name: String,
    /// GitHub's flag, which can disagree with the version string.
    #[serde(default)]
    pub prerelease: bool,
    #[serde(default)]
    pub html_url: String,
}

/// A newer release than the one running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Available {
    /// The version running, as the binary was built with it.
    pub current: String,
    /// The version found, spelled as SemVer plus an optional `-mac.N`.
    pub latest: String,
    /// That release's own page, or the releases page when it has none.
    pub url: String,
}

/// Direct download for this fork's arm64 DMG.
pub fn macos_dmg_url(version: &str) -> String {
    let version = version.trim().trim_start_matches('v');
    format!(
        "https://github.com/{GITHUB_REPO}/releases/download/v{version}/Lightforge-{version}-arm64.dmg"
    )
}

/// Fetch the release list, revalidating with `cache`.
///
/// Shares GitHub's 60/hour budget with the Proton check; a `304` is free.
pub async fn fetch(http: &Http, cache: &ETagCache) -> Result<Vec<Release>, UpdateError> {
    match http.get_json_cached(RELEASES_URL, cache).await? {
        Some(releases) => Ok(releases),
        None => Ok(cache.read_json()?),
    }
}

/// A published tag or a running version: upstream SemVer, then this fork's
/// `-mac.N` (zero when the string has no suffix).
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parsed {
    version: semver::Version,
    mac: u64,
}

impl Parsed {
    fn cmp_precedence(&self, other: &Self) -> std::cmp::Ordering {
        match self.version.cmp_precedence(&other.version) {
            std::cmp::Ordering::Equal => self.mac.cmp(&other.mac),
            other => other,
        }
    }

    fn display(&self) -> String {
        match self.mac {
            0 => self.version.to_string(),
            n => format!("{}-mac.{n}", self.version),
        }
    }
}

/// Tags are published as `v1.0.0-beta.9-mac.3`; SemVer does not want the `v`,
/// and `-mac.N` is ours, not a SemVer pre-release identifier.
fn version_of(tag: &str) -> Option<Parsed> {
    let bare = tag.trim().trim_start_matches('v');
    if let Some((base, mac)) = mac_suffix(bare) {
        return Some(Parsed {
            version: semver::Version::parse(base).ok()?,
            mac,
        });
    }
    // A broken `-mac.` suffix is not a version we can order: feeding it to
    // SemVer would treat `9-mac` as one identifier and put beta.10 behind it.
    if bare.contains("-mac.") {
        return None;
    }
    Some(Parsed {
        version: semver::Version::parse(bare).ok()?,
        mac: 0,
    })
}

/// `1.0.0-beta.9-mac.3` → (`1.0.0-beta.9`, 3).
fn mac_suffix(bare: &str) -> Option<(&str, u64)> {
    let (base, n) = bare.rsplit_once("-mac.")?;
    if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((base, n.parse().ok()?))
}

/// The tag or running version as the update check spells it, or `None`
/// when it cannot be ordered.
pub fn canonical_version(tag: &str) -> Option<String> {
    version_of(tag).map(|v| v.display())
}

/// The newest release worth telling someone running `current` about.
///
/// A prerelease build hears about anything newer; a stable build only about
/// stable releases. `None` when current, or when `current` is no version (a
/// development build).
pub fn newer_than(current: &str, releases: &[Release]) -> Option<Available> {
    let here = version_of(current)?;
    let wants_prereleases = !here.version.pre.is_empty();

    releases
        .iter()
        .filter_map(|release| version_of(&release.tag_name).map(|version| (version, release)))
        // Stable only if both the tag and GitHub's flag say so.
        .filter(|(version, release)| {
            wants_prereleases || (version.version.pre.is_empty() && !release.prerelease)
        })
        // `cmp_precedence`: `Ord` counts build metadata (`1.0.0+2` > `1.0.0`).
        .filter(|(version, _)| version.cmp_precedence(&here) == std::cmp::Ordering::Greater)
        .max_by(|(a, _), (b, _)| a.cmp_precedence(b))
        .map(|(version, release)| Available {
            current: here.display(),
            latest: version.display(),
            url: match release.html_url.is_empty() {
                true => RELEASES_PAGE.to_owned(),
                false => release.html_url.clone(),
            },
        })
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error(transparent)]
    Net(#[from] NetError),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(tag: &str, prerelease: bool) -> Release {
        Release {
            tag_name: tag.to_owned(),
            prerelease,
            html_url: format!("https://example.invalid/{tag}"),
        }
    }

    #[test]
    fn the_baked_version_is_cargo_or_a_mac_build_of_it() {
        let cargo = env!("CARGO_PKG_VERSION");
        let mac = VERSION.strip_prefix(&format!("{cargo}-mac."));
        assert!(
            VERSION == cargo
                || mac.is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())),
            "VERSION={VERSION} CARGO_PKG_VERSION={cargo}"
        );
    }

    #[test]
    fn the_check_looks_at_this_fork() {
        assert!(RELEASES_URL.contains("matt-dance/lightforge-mac"));
        assert!(RELEASES_PAGE.contains("matt-dance/lightforge-mac"));
        assert_eq!(
            macos_dmg_url("1.0.0-beta.9-mac.3"),
            "https://github.com/matt-dance/lightforge-mac/releases/download/v1.0.0-beta.9-mac.3/Lightforge-1.0.0-beta.9-mac.3-arm64.dmg"
        );
        assert_eq!(
            macos_dmg_url("v1.0.0-beta.9-mac.3"),
            macos_dmg_url("1.0.0-beta.9-mac.3"),
            "a leading v is the tag's, not the file's"
        );
    }

    #[test]
    fn a_beta_is_told_about_the_next_beta() {
        let releases = vec![
            release("v1.0.0-beta.3", true),
            release("v1.0.0-beta.2", true),
        ];
        let found = newer_than("1.0.0-beta.2", &releases).expect("beta.3 is newer");
        assert_eq!(found.latest, "1.0.0-beta.3");
        assert_eq!(found.url, "https://example.invalid/v1.0.0-beta.3");
    }

    /// The reason this cannot be a string compare, and the reason
    /// `releases/latest` cannot be used either.
    #[test]
    fn ten_is_newer_than_two_and_the_release_is_newer_than_both() {
        let releases = vec![
            release("v1.0.0-beta.2", true),
            release("v1.0.0-beta.10", true),
        ];
        assert_eq!(
            newer_than("1.0.0-beta.2", &releases).unwrap().latest,
            "1.0.0-beta.10",
            "string order would put beta.10 before beta.2"
        );

        let shipped = vec![release("v1.0.0", false), release("v1.0.0-beta.10", true)];
        assert_eq!(
            newer_than("1.0.0-beta.10", &shipped).unwrap().latest,
            "1.0.0"
        );
    }

    #[test]
    fn a_stable_build_is_not_pushed_at_prereleases() {
        let releases = vec![release("v1.1.0-beta.1", true), release("v1.0.0", false)];
        assert!(newer_than("1.0.0", &releases).is_none());

        // A tag that reads as stable but was published as a prerelease is
        // still a prerelease: either opinion is enough to hold it back.
        let mislabelled = vec![release("v1.1.0", true)];
        assert!(newer_than("1.0.0", &mislabelled).is_none());
        // And the same release does reach someone already on a beta.
        assert_eq!(
            newer_than("1.0.0-beta.1", &mislabelled).unwrap().latest,
            "1.1.0"
        );
    }

    #[test]
    fn the_newest_wins_whatever_order_the_list_arrives_in() {
        let releases = vec![
            release("v1.0.0-beta.1", true),
            release("v1.0.0-beta.9", true),
            release("v1.0.0-beta.4", true),
        ];
        assert_eq!(
            newer_than("1.0.0-beta.1", &releases).unwrap().latest,
            "1.0.0-beta.9"
        );
    }

    #[test]
    fn being_current_says_nothing_and_neither_does_nonsense() {
        let releases = vec![release("v1.0.0-beta.2", true)];
        assert!(
            newer_than("1.0.0-beta.2", &releases).is_none(),
            "already on it"
        );
        assert!(
            newer_than("1.0.0-beta.3", &releases).is_none(),
            "ahead of it"
        );
        assert!(newer_than("not-a-version", &releases).is_none());
        assert!(newer_than("1.0.0-beta.2", &[release("nightly", true)]).is_none());
        assert!(newer_than("1.0.0-beta.2", &[]).is_none());
    }

    /// Build metadata is not part of precedence, so a rebuild of the same
    /// version is not an update.
    #[test]
    fn build_metadata_is_not_a_new_version() {
        let releases = vec![release("v1.0.0-beta.2+20261013", true)];
        assert!(newer_than("1.0.0-beta.2", &releases).is_none());
    }

    #[test]
    fn a_later_mac_build_of_the_same_upstream_is_newer() {
        let releases = vec![
            release("v1.0.0-beta.9-mac.1", true),
            release("v1.0.0-beta.9-mac.3", true),
            release("v1.0.0-beta.9-mac.2", true),
        ];
        let found = newer_than("1.0.0-beta.9-mac.1", &releases).expect("mac.3 is newer");
        assert_eq!(found.latest, "1.0.0-beta.9-mac.3");
        assert_eq!(found.current, "1.0.0-beta.9-mac.1");
        assert!(newer_than("1.0.0-beta.9-mac.3", &releases).is_none());
        assert_eq!(
            newer_than("1.0.0-beta.9-mac.2", &releases).unwrap().latest,
            "1.0.0-beta.9-mac.3"
        );
    }

    #[test]
    fn mac_ten_is_newer_than_mac_two() {
        let releases = vec![release("v1.0.0-beta.9-mac.10", true)];
        assert_eq!(
            newer_than("1.0.0-beta.9-mac.2", &releases).unwrap().latest,
            "1.0.0-beta.9-mac.10",
            "string order would put mac.10 before mac.2"
        );
    }

    #[test]
    fn a_mac_build_is_newer_than_the_unsuffixed_upstream_tag() {
        let releases = vec![release("v1.0.0-beta.9-mac.1", true)];
        assert_eq!(
            newer_than("1.0.0-beta.9", &releases).unwrap().latest,
            "1.0.0-beta.9-mac.1",
            "the running binary must carry -mac.N or every release looks newer"
        );
    }

    #[test]
    fn a_newer_upstream_base_beats_any_mac_build_of_an_older_one() {
        let releases = vec![
            release("v1.0.0-beta.9-mac.99", true),
            release("v1.0.0-beta.10-mac.1", true),
            release("v1.0.0-beta.10", true),
        ];
        assert_eq!(
            newer_than("1.0.0-beta.9-mac.99", &releases).unwrap().latest,
            "1.0.0-beta.10-mac.1",
            "plain SemVer would put beta.10 behind 9-mac"
        );
        assert_eq!(
            newer_than("1.0.0-beta.10", &releases).unwrap().latest,
            "1.0.0-beta.10-mac.1"
        );
        assert!(newer_than("1.0.0-beta.10-mac.1", &releases).is_none());
    }

    #[test]
    fn a_broken_mac_suffix_is_skipped_not_ordered_as_semver() {
        let releases = vec![release("v1.0.0-beta.9-mac.notanumber", true)];
        assert!(newer_than("1.0.0-beta.9", &releases).is_none());
        assert_eq!(
            canonical_version("v1.0.0-beta.9-mac.3").as_deref(),
            Some("1.0.0-beta.9-mac.3")
        );
        assert_eq!(
            canonical_version("1.0.0-beta.9").as_deref(),
            Some("1.0.0-beta.9")
        );
        assert!(canonical_version("nightly").is_none());
    }
}
