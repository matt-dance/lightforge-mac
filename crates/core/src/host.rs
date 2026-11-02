//! What this build launches, and what it does not.
//!
//! Linux runs the Windows client through Proton. macOS downloads the OSX
//! TACT slice and starts `World of Warcraft.app` itself. The rest of the
//! crate branches on these two facts rather than scattering `cfg` at every
//! call site.

/// Whether launches go through Proton and a Wine prefix.
pub const fn uses_proton() -> bool {
    cfg!(not(target_os = "macos"))
}

/// Whether this binary is the native macOS build.
pub const fn is_macos() -> bool {
    cfg!(target_os = "macos")
}
