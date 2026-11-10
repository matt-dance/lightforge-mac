//! Asking the desktop session for things: no sleep while the store is being
//! written, and a word when a download ends out of sight.

use std::sync::Arc;

/// Holds the machine awake until dropped. Linux: a logind inhibit fd.
/// macOS: a `caffeinate` child killed on drop.
pub struct SleepLock {
    #[cfg(target_os = "linux")]
    _fd: Arc<std::os::fd::OwnedFd>,
    #[cfg(target_os = "macos")]
    child: std::sync::Mutex<Option<std::process::Child>>,
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    _unused: (),
}

impl std::fmt::Debug for SleepLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SleepLock")
    }
}

/// A folder, through the desktop portal (Linux) or a native dialog (macOS).
/// Linux uses `ashpd`; `rfd` there talks to libdbus in-process and never
/// returns. See `pick.rs`.
#[cfg(target_os = "linux")]
pub async fn pick_folder() -> Option<std::path::PathBuf> {
    let request = ashpd::desktop::file_chooser::OpenFileRequest::default()
        .title("Choose a folder for the Wine prefix")
        .modal(true)
        .multiple(false)
        .directory(true);
    let files = request.send().await.ok()?.response().ok()?;
    files.uris().first().and_then(|uri| uri.to_file_path().ok())
}

#[cfg(target_os = "macos")]
pub async fn pick_folder() -> Option<std::path::PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Choose a folder for the Wine prefix")
        .pick_folder()
        .await
        .map(|file| file.path().to_path_buf())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn pick_folder() -> Option<std::path::PathBuf> {
    None
}

impl Drop for SleepLock {
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        if let Ok(mut child) = self.child.lock() {
            if let Some(mut child) = child.take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Hold off suspend, an idle one included, until the lock is dropped.
pub async fn keep_awake(why: &'static str) -> Result<Arc<SleepLock>, String> {
    #[cfg(target_os = "linux")]
    {
        linux_keep_awake(why).await
    }
    #[cfg(target_os = "macos")]
    {
        macos_keep_awake(why)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = why;
        Err("no sleep lock on this OS".to_owned())
    }
}

#[cfg(target_os = "linux")]
async fn linux_keep_awake(why: &'static str) -> Result<Arc<SleepLock>, String> {
    use ashpd::zbus::{self, zvariant};
    use std::os::fd::OwnedFd;

    let system = zbus::Connection::system().await.map_err(|e| format!("system bus: {e}"))?;
    let reply = system
        .call_method(
            Some("org.freedesktop.login1"),
            "/org/freedesktop/login1",
            Some("org.freedesktop.login1.Manager"),
            "Inhibit",
            &("sleep", "Lightforge", why, "block"),
        )
        .await
        .map_err(|e| format!("logind refused the sleep lock: {e}"))?;
    let lock: zvariant::OwnedFd = reply.body().deserialize().map_err(|e| format!("logind's reply: {e}"))?;
    let fd: OwnedFd = lock.into();
    Ok(Arc::new(SleepLock { _fd: Arc::new(fd) }))
}

#[cfg(target_os = "macos")]
fn macos_keep_awake(why: &'static str) -> Result<Arc<SleepLock>, String> {
    let child = std::process::Command::new("caffeinate")
        .args(["-dimsu", "-w"])
        .arg(std::process::id().to_string())
        .spawn()
        .map_err(|e| format!("caffeinate: {e}"))?;
    tracing::info!("holding sleep with caffeinate ({why})");
    Ok(Arc::new(SleepLock {
        child: std::sync::Mutex::new(Some(child)),
    }))
}

/// Show a desktop notification. Best effort: a session without a
/// notification server (gamescope) just goes without.
pub async fn notify(summary: String, body: String) {
    #[cfg(target_os = "linux")]
    {
        linux_notify(summary, body).await;
    }
    #[cfg(target_os = "macos")]
    {
        macos_notify(&summary, &body);
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        tracing::info!("notify: {summary}: {body}");
    }
}

#[cfg(target_os = "linux")]
async fn linux_notify(summary: String, body: String) {
    use ashpd::zbus::{self, zvariant};
    use std::collections::HashMap;

    let sent = async {
        let session = zbus::Connection::session().await?;
        let hints = HashMap::from([("desktop-entry", zvariant::Value::from(crate::APP_ID))]);
        session
            .call_method(
                Some("org.freedesktop.Notifications"),
                "/org/freedesktop/Notifications",
                Some("org.freedesktop.Notifications"),
                "Notify",
                &("Lightforge", 0u32, crate::APP_ID, summary.as_str(), body.as_str(), Vec::<&str>::new(), hints, -1i32),
            )
            .await
            .map(drop)
    };
    if let Err(e) = sent.await {
        tracing::warn!("no desktop notification for \"{summary}\": {e}");
    }
}

#[cfg(target_os = "macos")]
fn macos_notify(summary: &str, body: &str) {
    let script = format!(
        "display notification \"{}\" with title \"{}\"",
        body.replace('\\', "\\\\").replace('"', "\\\""),
        summary.replace('\\', "\\\\").replace('"', "\\\""),
    );
    if let Err(e) = std::process::Command::new("osascript")
        .args(["-e", &script])
        .status()
    {
        tracing::warn!("no desktop notification for \"{summary}\": {e}");
    }
}
