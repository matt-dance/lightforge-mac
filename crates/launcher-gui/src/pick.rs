//! Native file dialogs. Linux uses the XDG portal (`ashpd`); macOS uses
//! `rfd`. `rfd` on Linux talks to libdbus in-process and never returns, so
//! it stays off that target.

use std::path::PathBuf;

#[cfg(target_os = "linux")]
pub async fn pick_image() -> Option<PathBuf> {
    let request = ashpd::desktop::file_chooser::OpenFileRequest::default()
        .title("Choose a screenshot")
        .modal(true)
        .multiple(false)
        .filter(
            ashpd::desktop::file_chooser::FileFilter::new("Images")
                .mimetype("image/png")
                .mimetype("image/jpeg")
                .mimetype("image/webp")
                .mimetype("image/bmp"),
        );
    let files = request.send().await.ok()?.response().ok()?;
    files.uris().first().and_then(|uri| uri.to_file_path().ok())
}

#[cfg(target_os = "macos")]
pub async fn pick_image() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Choose a screenshot")
        .add_filter("Images", &["png", "jpg", "jpeg", "webp", "bmp"])
        .pick_file()
        .await
        .map(|file| file.path().to_path_buf())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn pick_image() -> Option<PathBuf> {
    None
}

#[cfg(target_os = "linux")]
pub async fn pick_folder() -> Option<PathBuf> {
    let request = ashpd::desktop::file_chooser::OpenFileRequest::default()
        .title("Choose a Wine prefix or a game folder")
        .modal(true)
        .directory(true)
        .multiple(false);
    let files = request.send().await.ok()?.response().ok()?;
    files.uris().first().and_then(|uri| uri.to_file_path().ok())
}

#[cfg(target_os = "macos")]
pub async fn pick_folder() -> Option<PathBuf> {
    rfd::AsyncFileDialog::new()
        .set_title("Choose a game folder")
        .pick_folder()
        .await
        .map(|file| file.path().to_path_buf())
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub async fn pick_folder() -> Option<PathBuf> {
    None
}
