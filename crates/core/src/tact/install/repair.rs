//! Re-extract the install-manifest files and repair macOS bundles.
//!
//! The CASC store is not opened for writing. An 85 GB Forever install is
//! left alone; only the loose `.app` / CEF files (tens to hundreds of MB)
//! are fetched again.

use std::path::Path;

use super::prepare::prepare;
use super::{io, Event, InstallError, Options};
use crate::net::Http;
use crate::product::ProductCode;
use crate::tact::download::{extract_install, write_flavor_info, Cancel, Catalog, Source};

/// Re-extract the install-manifest files into `dest`'s flavor folder and
/// run the macOS launch fixup (framework assembly, `+x`, ad-hoc sign).
///
/// Does not create a [`crate::tact::writer::StoreWriter`], so it cannot
/// rewrite or truncate the store. `.build.info` is left as it is.
pub async fn repair_install_files(
    http: &Http,
    dest: &Path,
    code: &ProductCode,
    options: &Options,
    cancel: &Cancel,
    on_event: &mut impl FnMut(Event),
) -> Result<crate::macos_install::Report, InstallError> {
    let mut options = options.clone();
    options.install_files = true;
    options.stage = false;
    // A repair of an existing flavor must not pick a new folder name.
    if options.flavor.is_empty() {
        options.flavor = crate::paths::flavor_dir_name(dest, code);
    }

    let prepared = prepare(http, dest, code, &options, on_event).await?;
    let flavor_dir = dest.join(&prepared.flavor);
    let selected = prepared.install.select(&prepared.tags);
    let catalog = Catalog {
        encoding: &prepared.encoding,
        locations: &prepared.locations,
        loose: &prepared.loose,
    };
    let source = Source { http, client: &prepared.client };
    let extracted = extract_install(
        source,
        &selected,
        catalog,
        &flavor_dir,
        cancel,
        |e| on_event(Event::InstallFile { path: e.path.clone(), bytes: e.bytes }),
    )
    .await?;
    on_event(Event::Extracted { files: extracted.written.len(), bytes: extracted.bytes() });
    for (path, reason) in &extracted.failed {
        tracing::warn!("install file {path} failed: {reason}");
    }

    write_flavor_info(&flavor_dir, code).map_err(io(&flavor_dir))?;

    // Assemble / sign even when a handful of files failed: a missing locale
    // plist must not leave BlizzardBrowser un-signed.
    let mut report = crate::macos_install::prepare_tree(&flavor_dir);
    report.extracted = extracted.written.len();
    report.extract_failed = extracted.failed.len();
    Ok(report)
}
