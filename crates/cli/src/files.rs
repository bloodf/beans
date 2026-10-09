//! Attachments: files a Device sends with a message. The bytes leave as a `file` blob under
//! the attachment's id, encrypted with the account key like a chat op, and land in
//! `~/.beans-v2/files/<id>` on every Device that needs them: the Runner copies them into the
//! bot's working directory for its turn, and the app shows them in the transcript.

use std::path::{Path, PathBuf};
use std::sync::Arc;

#[cfg(feature = "runner")]
use beans_agent::ContentPart;

use crate::app::App;
use crate::model::Attachment;

pub const MAX_ATTACHMENT_BYTES: u64 = 100 * 1024 * 1024;

/// A file the app asked to send: a path on this machine, with the id the app already shows.
#[derive(Debug, Clone, serde::Deserialize)]
pub struct OutgoingFile {
    pub path: String,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    /// The app's own guesses, kept when given so the bubble it already shows does not change.
    #[serde(default)]
    pub mime: Option<String>,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

pub fn new_id() -> String {
    let mut bytes = [0u8; 6];
    rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut bytes);
    format!("att-{}", bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
}

pub fn local_path(app: &App, id: &str) -> PathBuf {
    app.config.files_dir().join(id)
}

pub fn is_local(app: &App, id: &str) -> bool {
    local_path(app, id).is_file()
}

/// Stores one file through the same preparation boundary as a chat's attachment batch.
pub fn store(app: &App, file: &OutgoingFile) -> anyhow::Result<Attachment> {
    Ok(store_many(app, std::slice::from_ref(file))?.remove(0))
}

/// Prepare every source in a private staging directory before replacing any stored bytes.
/// A source failure drops the staged batch, including writes reusing an existing attachment id.
pub fn store_many(app: &App, files: &[OutgoingFile]) -> anyhow::Result<Vec<Attachment>> {
    if files.is_empty() { return Ok(Vec::new()); }
    let mut staged=prepare_many(app,files)?;
    staged.finalize(app)?;
    staged.accepted=true;
    Ok(staged.attachments.clone())
}

pub(crate) struct StagedFiles {
    staging: tempfile::TempDir,
    pub attachments: Vec<Attachment>,
    installed: Vec<(PathBuf, Option<PathBuf>)>,
    accepted: bool,
}

pub(crate) fn prepare_many(app: &App, files: &[OutgoingFile]) -> anyhow::Result<StagedFiles> {
    let staging=tempfile::tempdir_in(&app.config.home)?;
    let mut attachments=Vec::with_capacity(files.len());
    for file in files { attachments.push(store_at(staging.path(),file)?); }
    Ok(StagedFiles {staging,attachments,installed:Vec::new(),accepted:false})
}

impl StagedFiles {
    pub(crate) fn outbox(&self, app: &App, chat: &str) -> anyhow::Result<Vec<crate::app::OutboxItem>> {
        let Some(dek)=app.dek() else {return Ok(Vec::new())};
        self.attachments.iter().map(|attachment| Ok(crate::app::OutboxItem {
            id:attachment.id.clone(),kind:"file".into(),recipient:None,
            ciphertext:crate::crypto::encrypt(&dek,"file",&std::fs::read(self.staging.path().join(&attachment.id))?)?,
            slot:None,group:Some(crate::model::relay_name(chat)),
        })).collect()
    }
    pub(crate) fn finalize(&mut self, app: &App) -> anyhow::Result<()> {
        if self.attachments.is_empty() {return Ok(())}
        let dir=app.config.files_dir(); std::fs::create_dir_all(&dir)?; crate::config::set_private(&dir)?;
        let ids:std::collections::HashSet<_>=self.attachments.iter().map(|a|a.id.clone()).collect();
        for id in ids {
            let destination=dir.join(&id);
            anyhow::ensure!(!destination.exists() || destination.is_file(), "Attachment destination is not a file");
            let backup=if destination.exists() {
                let backup=self.staging.path().join(format!("backup-{id}"));
                std::fs::rename(&destination,&backup)?; Some(backup)
            } else {None};
            self.installed.push((destination.clone(),backup));
            std::fs::rename(self.staging.path().join(id),destination)?;
        }
        Ok(())
    }
    pub(crate) fn accept(&mut self) {self.accepted=true;}
}

impl Drop for StagedFiles {
    fn drop(&mut self) {
        if self.accepted {return}
        for (destination,backup) in self.installed.iter().rev() {
            let _=std::fs::remove_file(destination);
            if let Some(backup)=backup {
                if let Err(error)=std::fs::rename(backup,destination) {
                    tracing::error!(%error, path=%destination.display(), "Restoring rejected attachment failed; retaining staging backup");
                    let _=self.staging.disable_cleanup(true);
                }
            }
        }
    }
}

fn store_at(dir: &Path, file: &OutgoingFile) -> anyhow::Result<Attachment> {
    let source = Path::new(&file.path);
    let metadata = std::fs::metadata(source)?;
    if !metadata.is_file() {
        anyhow::bail!("{} is not a file", file.path);
    }
    if metadata.len() > MAX_ATTACHMENT_BYTES {
        anyhow::bail!("{} is larger than {} MB", file.path, MAX_ATTACHMENT_BYTES / 1024 / 1024);
    }
    let id = match &file.id {
        Some(id) if id.starts_with("att-") && id.len() <= 48 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') => id.clone(),
        _ => new_id(),
    };
    let name = file
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "file".into());
    let mime = file.mime.clone().filter(|m| m.contains('/')).unwrap_or_else(|| mime_for(&name).to_string());
    let bytes = std::fs::read(source)?;
    write_at(dir, &id, &bytes)?;
    let (width, height) = match (file.width, file.height) {
        (Some(w), Some(h)) => (Some(w), Some(h)),
        _ if mime.starts_with("image/") => image_size(&bytes),
        _ => (None, None),
    };
    Ok(Attachment { id, name, mime, size: metadata.len(), width, height })
}

fn write_local(app: &App, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    write_at(&app.config.files_dir(), id, bytes)
}

fn write_at(dir: &Path, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    std::fs::create_dir_all(&dir)?;
    crate::config::set_private(&dir)?;
    let path = dir.join(id);
    std::fs::write(&path, bytes)?;
    crate::config::set_private(&path)?;
    Ok(())
}

/// Queues the attachment's bytes as a `file` blob. Call before the chat op that names it, so
/// the relay hands Runners the bytes before the message that needs them.
pub fn push_blob(app: &App, chat_id: Option<&str>, attachment: &Attachment) -> anyhow::Result<()> {
    let Some(dek) = app.dek() else { return Ok(()) };
    let bytes = std::fs::read(local_path(app, &attachment.id))?;
    let ciphertext = crate::crypto::encrypt(&dek, "file", &bytes)?;
    app.push_file_blob(attachment.id.clone(), chat_id, ciphertext);
    Ok(())
}

/// The attachment's bytes on this machine, fetched from the relay when another Device sent it.
pub async fn ensure_local(app: &Arc<App>, attachment: &Attachment) -> anyhow::Result<PathBuf> {
    let path = local_path(app, &attachment.id);
    if path.is_file() {
        return Ok(path);
    }
    let url = app.relay_url().ok_or_else(|| anyhow::anyhow!("no relay configured"))?;
    let machine_file = app.machine_file().ok_or_else(|| anyhow::anyhow!("not paired"))?;
    let machine = machine_file.machine()?;
    let dek = machine_file.dek()?;
    let token = crate::sync::token_or_register(app, &url, &machine).await.map_err(|e| anyhow::anyhow!(e.message))?;
    let ciphertext = app
        .relay
        .get_file(&url, &token, &attachment.id)
        .await
        .map_err(|e| anyhow::anyhow!(e.message))?
        .ok_or_else(|| anyhow::anyhow!("the relay no longer has {}", attachment.name))?;
    let bytes = crate::crypto::decrypt(&dek, "file", &ciphertext)?;
    write_local(app, &attachment.id, &bytes)?;
    Ok(path)
}

/// Fetches whatever a turn's transcript refers to that is not here yet. Failures are logged;
/// the turn still runs, and the prompt says the file could not be fetched.
pub async fn prefetch(app: &Arc<App>, attachments: &[Attachment]) {
    for attachment in attachments {
        if is_local(app, &attachment.id) {
            continue;
        }
        if let Err(error) = ensure_local(app, attachment).await {
            tracing::warn!(%error, name = %attachment.name, "fetching an attachment");
        }
    }
}

/// The path a bot reads the attachment at: `<workdir>/attachments/<id>/<name>`, copied from the
/// store on first use. Stable across turns, so the transcript names the same path each time.
pub fn materialize(app: &App, attachment: &Attachment, workdir: &Path) -> Option<PathBuf> {
    let source = local_path(app, &attachment.id);
    if !source.is_file() {
        return None;
    }
    let dir = workdir.join("attachments").join(&attachment.id);
    let target = dir.join(safe_name(&attachment.name));
    if !target.is_file() {
        if let Err(error) = std::fs::create_dir_all(&dir).and_then(|_| std::fs::copy(&source, &target).map(|_| ())) {
            tracing::warn!(%error, name = %attachment.name, "copying an attachment into the workspace");
            return None;
        }
    }
    Some(target)
}

/// What the model sees for one attachment: a line naming the file and, when `pixels`,
/// a prepared image part or a line saying why the image cannot be shown.
#[cfg(feature = "runner")]
pub fn content_parts(app: &App, attachment: &Attachment, workdir: &Path, pixels: bool) -> Vec<ContentPart> {
    let Some(path) = materialize(app, attachment, workdir) else {
        return vec![ContentPart::text(format!("[Attachment {} ({}) could not be fetched on this Runner]", attachment.name, attachment.mime))];
    };
    let mut parts = vec![ContentPart::text(format!(
        "[Attached: {} ({}, {}) at {}]",
        attachment.name,
        attachment.mime,
        human_size(attachment.size),
        path.display()
    ))];
    if pixels && attachment.is_image() {
        match inline_image(app, attachment) {
            Ok(image) => parts.extend(image.into_parts()),
            Err(why) => parts.push(ContentPart::text(format!("[{} is not shown as an image: {why}]", attachment.name))),
        }
    }
    parts
}

/// Model-ready image cache stays local beside encrypted attachment bytes, never on relay.
#[cfg(feature = "runner")]
fn inline_image_path(app: &App, id: &str) -> PathBuf {
    app.config.files_dir().join(format!("{id}.inline.json"))
}

/// The image an attachment shows a model (`images::prepare`): a HEIC photo converted on a Mac, a
/// rotated one turned upright, a large one scaled down with a note, or why it cannot go. Made
/// once and kept beside the attachment, failures too, since every turn builds its messages again
/// and a photo takes a moment to make.
#[cfg(feature = "runner")]
pub fn inline_image(app: &App, attachment: &Attachment) -> Result<beans_agent::images::Inline, String> {
    let kept = inline_image_path(app, &attachment.id);
    if let Some(made) = crate::config::read_json::<Result<beans_agent::images::Inline, String>>(&kept) {
        return made;
    }
    let bytes = std::fs::read(local_path(app, &attachment.id)).map_err(|e| format!("it could not be read: {e}"))?;
    let made = beans_agent::images::prepare(&bytes);
    if let Err(error) = crate::config::write_json_private(&kept, &made) {
        tracing::warn!(%error, name = %attachment.name, "keeping an attachment's image for the model");
    }
    made
}

/// Decode before building transcript, off async threads.
#[cfg(feature = "runner")]
pub async fn make_images(app: &Arc<App>, attachments: &[Attachment]) {
    let pending: Vec<Attachment> = attachments.iter().filter(|attachment| attachment.is_image() && is_local(app, &attachment.id) && !inline_image_path(app, &attachment.id).is_file()).cloned().collect();
    if pending.is_empty() {
        return;
    }
    let app = app.clone();
    let made = tokio::task::spawn_blocking(move || {
        for attachment in &pending {
            let _ = inline_image(&app, attachment);
        }
    });
    if let Err(error) = made.await {
        tracing::warn!(%error, "making attachments' images for the model");
    }
}

fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() { "file".into() } else { trimmed.to_string() }
}

pub fn human_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    }
}

pub fn mime_for(name: &str) -> &'static str {
    let extension = name.rsplit('.').next().map(|e| e.to_ascii_lowercase()).unwrap_or_default();
    match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "heic" => "image/heic",
        "svg" => "image/svg+xml",
        "pdf" => "application/pdf",
        "txt" | "log" => "text/plain",
        "md" | "markdown" => "text/markdown",
        "json" => "application/json",
        "csv" => "text/csv",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "mp3" => "audio/mpeg",
        "m4a" => "audio/mp4",
        "wav" => "audio/wav",
        "mp4" => "video/mp4",
        "mov" => "video/quicktime",
        _ => "application/octet-stream",
    }
}

/// Pixel size from the header of a PNG, JPEG, GIF, or WebP; the apps size thumbnails from it
/// without decoding the file.
fn image_size(bytes: &[u8]) -> (Option<u32>, Option<u32>) {
    fn be32(b: &[u8]) -> u32 {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
    if bytes.len() >= 24 && bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        return (Some(be32(&bytes[16..20])), Some(be32(&bytes[20..24])));
    }
    if bytes.len() >= 10 && bytes.starts_with(b"GIF8") {
        return (Some(u16::from_le_bytes([bytes[6], bytes[7]]) as u32), Some(u16::from_le_bytes([bytes[8], bytes[9]]) as u32));
    }
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" && &bytes[12..16] == b"VP8X" {
        let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
        let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
        return (Some(w), Some(h));
    }
    if bytes.len() > 4 && bytes.starts_with(&[0xFF, 0xD8]) {
        let mut i = 2;
        while i + 9 < bytes.len() {
            if bytes[i] != 0xFF {
                i += 1;
                continue;
            }
            let marker = bytes[i + 1];
            if (0xC0..=0xCF).contains(&marker) && marker != 0xC4 && marker != 0xC8 && marker != 0xCC {
                let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
                let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
                return (Some(w), Some(h));
            }
            let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
            i += 2 + len.max(2);
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_header_size() {
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size(&png), (Some(640), Some(480)));
        assert_eq!(image_size(b"nope"), (None, None));
    }

    #[cfg(feature = "runner")]
    #[test]
    fn attachment_image_uses_decoded_type_and_size_bounds() {
        use base64::Engine;
        let home = std::env::temp_dir().join(format!("beans-files-{}", uuid::Uuid::new_v4()));
        let app = App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap();
        let image = image::DynamicImage::ImageRgb8(image::ImageBuffer::from_fn(2600, 100, |x, y| image::Rgb([(x % 256) as u8, (y % 256) as u8, 60])));
        let mut png = Vec::new();
        image.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        write_local(&app, "image-1", &png).unwrap();
        let attachment = Attachment { id: "image-1".into(), name: "wrong.jpg".into(), mime: "image/jpeg".into(), size: png.len() as u64, width: None, height: None };
        let parts = content_parts(&app, &attachment, &home.join("work"), true);
        assert!(parts[1].as_text().unwrap().contains("displayed at 2000x77"));
        let ContentPart::Image { data, mime_type } = &parts[2] else { panic!("missing image") };
        assert!(matches!(mime_type.as_str(), "image/png" | "image/jpeg"), "actual decoded bytes decide type");
        assert_eq!(image::load_from_memory(&base64::engine::general_purpose::STANDARD.decode(data).unwrap()).unwrap().width(), 2000);
        assert!(app.config.files_dir().join("image-1.inline.json").is_file());
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn mime_and_names() {
        assert_eq!(mime_for("Photo.JPG"), "image/jpeg");
        assert_eq!(mime_for("notes"), "application/octet-stream");
        assert_eq!(safe_name("../../etc/passwd"), "_.._etc_passwd");
        assert_eq!(human_size(2_500_000), "2.4 MB");
    }

    #[test]
    fn text_body_without_attachments_still_parses() {
        let body: crate::model::Body = serde_json::from_str(r#"{"kind":"text","text":"hi"}"#).unwrap();
        assert_eq!(body, crate::model::Body::text("hi"));
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"kind":"text","text":"hi"}"#);
    }
}
