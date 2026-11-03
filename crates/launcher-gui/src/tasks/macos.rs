//! Repair a macOS install in place: re-extract the `.app` / CEF tree, then
//! assemble frameworks and ad-hoc sign. The CASC store is not rewritten.

use super::*;

/// Re-extract install-manifest files when we can reach the CDN; always run
/// the local bundle fixup so a flaky host still gets `Versions/Current` and
/// `+x`.
///
/// The TACT prepare future is not `Send` (the same reason downloads run on
/// their own thread), so this hops to a blocking thread with its own runtime.
pub async fn repair_macos(
    install: PathBuf,
    code: ProductCode,
    region: Region,
    locale: String,
) -> Result<lightforge_core::macos_install::Report, String> {
    tokio::task::spawn_blocking(move || repair_macos_blocking(install, code, region, locale))
        .await
        .map_err(|e| e.to_string())?
}

fn repair_macos_blocking(
    install: PathBuf,
    code: ProductCode,
    region: Region,
    locale: String,
) -> Result<lightforge_core::macos_install::Report, String> {
    if !lightforge_core::host::is_macos() {
        return Ok(lightforge_core::macos_install::prepare_install(&install));
    }

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async move {
        let http = match Http::new() {
            Ok(http) => http,
            Err(e) => {
                tracing::warn!("CDN repair unavailable ({e}); local bundle fixup only");
                return Ok(lightforge_core::macos_install::prepare_install(&install));
            }
        };
        let options = InstallOptions {
            region,
            locale,
            flavor: lightforge_core::paths::flavor_dir_name(&install, &code),
            ..InstallOptions::default()
        };
        let cancel = Cancel::new();
        match install::repair_install_files(
            &http,
            &install,
            &code,
            &options,
            &cancel,
            &mut ignore_event,
        )
        .await
        {
            Ok(report) => Ok(report),
            Err(e) => {
                tracing::warn!("CDN repair failed ({}); local bundle fixup only", chain(&e));
                Ok(lightforge_core::macos_install::prepare_install(&install))
            }
        }
    })
}

fn ignore_event(_event: Event) {}
