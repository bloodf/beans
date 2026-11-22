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

/// Attachment ids arrive from other Devices inside synced chat ops and become file names under
/// `files/`, so every path built from one goes through this check first. Clients make
/// `att-<hex>` ids; older tests and Devices used other plain ids (`image-1`, uuids), which stay
/// valid. A dot is never allowed: it would permit `..` and clash with `<id>.inline.json`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Where the attachment's bytes live on this machine; refuses ids that are not plain names.
pub fn local_path(app: &App, id: &str) -> anyhow::Result<PathBuf> {
    anyhow::ensure!(valid_id(id), "Attachment id is not valid");
    Ok(app.config.files_dir().join(id))
}

pub fn is_local(app: &App, id: &str) -> bool {
    local_path(app, id).is_ok_and(|path| path.is_file())
}

/// Whether the bytes at `path` can be the attachment's: a regular file, and the size the sender
/// declared when it declared one (older senders and mocks send 0).
fn matches_size(path: &Path, attachment: &Attachment) -> bool {
    std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file() && (attachment.size == 0 || metadata.len() == attachment.size))
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
    let mut attachments:Vec<Attachment>=Vec::with_capacity(files.len());
    for file in files {
        let attachment=store_at(staging.path(),file)?;
        // A second file under the same id would overwrite the first one's staged bytes.
        anyhow::ensure!(!attachments.iter().any(|a|a.id==attachment.id), "Two attachments in one message share the id {}", attachment.id);
        attachments.push(attachment);
    }
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
            std::fs::rename(self.staging.path().join(&id),destination)?;
            // New bytes under an old id: the model's prepared image was made from the old ones.
            #[cfg(feature = "runner")]
            let _=std::fs::remove_file(inline_image_path(app,&id));
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
        Some(id) if id.starts_with("att-") && id.len() <= 48 && valid_id(id) && !id.contains('_') => id.clone(),
        _ => new_id(),
    };
    let name = file
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| source.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "file".into());
    let mime = file.mime.clone().filter(|m| m.contains('/')).unwrap_or_else(|| mime_for(&name).to_string());
    // The file may grow after `metadata`: read at most one byte past the cap, and report the
    // size of what was actually read.
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut std::io::Read::take(std::fs::File::open(source)?, MAX_ATTACHMENT_BYTES + 1), &mut bytes)?;
    if bytes.len() as u64 > MAX_ATTACHMENT_BYTES {
        anyhow::bail!("{} is larger than {} MB", file.path, MAX_ATTACHMENT_BYTES / 1024 / 1024);
    }
    write_at(dir, &id, &bytes)?;
    let (width, height) = match (file.width, file.height) {
        (Some(w), Some(h)) => (Some(w), Some(h)),
        _ if mime.starts_with("image/") => image_size(&bytes),
        _ => (None, None),
    };
    Ok(Attachment { id, name, mime, size: bytes.len() as u64, width, height })
}

#[cfg_attr(not(feature = "runner"), allow(dead_code))]
fn write_local(app: &App, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    anyhow::ensure!(valid_id(id), "Attachment id is not valid");
    write_at(&app.config.files_dir(), id, bytes)
}

/// Atomic, private write: a reader never sees half a file, so a later `is_file` can trust it.
fn write_at(dir: &Path, id: &str, bytes: &[u8]) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir)?;
    crate::config::set_private(dir)?;
    crate::config::write_private(&dir.join(id), bytes)
}

/// Queues the attachment's bytes as a `file` blob. Call before the chat op that names it, so
/// the relay hands Runners the bytes before the message that needs them.
pub fn push_blob(app: &App, chat_id: Option<&str>, attachment: &Attachment) -> anyhow::Result<()> {
    let Some(dek) = app.dek() else { return Ok(()) };
    let bytes = std::fs::read(local_path(app, &attachment.id)?)?;
    let ciphertext = crate::crypto::encrypt(&dek, "file", &bytes)?;
    app.push_file_blob(attachment.id.clone(), chat_id, ciphertext);
    Ok(())
}

/// The attachment's bytes on this machine, fetched from the relay when another Device sent it.
pub async fn ensure_local(app: &Arc<App>, attachment: &Attachment) -> anyhow::Result<PathBuf> {
    let path = local_path(app, &attachment.id)?;
    if matches_size(&path, attachment) {
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
    if attachment.size != 0 && bytes.len() as u64 != attachment.size {
        anyhow::bail!("{} arrived with {} bytes instead of {}", attachment.name, bytes.len(), attachment.size);
    }
    write_local(app, &attachment.id, &bytes)?;
    // Semantic search skipped it while it was not here; now its messages can be indexed.
    if attachment.is_image() {
        if let Err(error) = app.store.requeue_picture(&attachment.id) {
            tracing::warn!(%error, "queueing a fetched picture for search");
        }
    }
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
    let source = local_path(app, &attachment.id).ok()?;
    if !matches_size(&source, attachment) {
        return None;
    }
    let dir = workdir.join("attachments").join(&attachment.id);
    let target = dir.join(safe_name(&attachment.name));
    if !is_current_copy(&source, &target) {
        // Copy beside the target and rename, so a bot never reads half a file; a resend under the
        // same id replaces the copy an earlier turn made.
        let partial = dir.join(format!(".{}.partial", attachment.id));
        let copied = std::fs::create_dir_all(&dir)
            .and_then(|_| std::fs::copy(&source, &partial))
            .and_then(|_| std::fs::rename(&partial, &target));
        if let Err(error) = copied {
            let _ = std::fs::remove_file(&partial);
            tracing::warn!(%error, name = %attachment.name, "copying an attachment into the workspace");
            return None;
        }
    }
    Some(target)
}

/// A workspace copy is current when it has the stored bytes' size and is not older than them.
fn is_current_copy(source: &Path, target: &Path) -> bool {
    let (Ok(source), Ok(target)) = (std::fs::metadata(source), std::fs::metadata(target)) else { return false };
    target.is_file()
        && source.len() == target.len()
        && match (source.modified(), target.modified()) {
            (Ok(source), Ok(target)) => target >= source,
            _ => true,
        }
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
    if !valid_id(&attachment.id) {
        return Err("its id is not valid".into());
    }
    let kept = inline_image_path(app, &attachment.id);
    if let Some(made) = crate::config::read_json::<Result<beans_agent::images::Inline, String>>(&kept) {
        return made;
    }
    let path = local_path(app, &attachment.id).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(path).map_err(|e| format!("it could not be read: {e}"))?;
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

/// Longest file name kept in a workspace, in bytes; file systems stop at 255.
const MAX_NAME_BYTES: usize = 200;

fn safe_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c == '/' || c == '\\' || c.is_control() { '_' } else { c }).collect();
    let trimmed = cleaned.trim().trim_start_matches('.');
    if trimmed.is_empty() {
        return "file".into();
    }
    if trimmed.len() <= MAX_NAME_BYTES {
        return trimmed.to_string();
    }
    // Shorten the stem and keep a short extension, so the bot still sees what kind of file it is.
    let extension = trimmed.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty() && e.len() <= 16).unwrap_or("");
    let budget = MAX_NAME_BYTES - if extension.is_empty() { 0 } else { extension.len() + 1 };
    let mut end = budget;
    while !trimmed.is_char_boundary(end) {
        end -= 1;
    }
    if extension.is_empty() { trimmed[..end].to_string() } else { format!("{}.{extension}", &trimmed[..end]) }
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
    let extension = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
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
    if bytes.len() >= 30 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        match &bytes[12..16] {
            b"VP8X" => {
                let w = 1 + u32::from_le_bytes([bytes[24], bytes[25], bytes[26], 0]);
                let h = 1 + u32::from_le_bytes([bytes[27], bytes[28], bytes[29], 0]);
                return (Some(w), Some(h));
            }
            // Lossy: a key frame's start code, then 14-bit width and height.
            b"VP8 " if bytes[23..26] == [0x9D, 0x01, 0x2A] => {
                let w = u16::from_le_bytes([bytes[26], bytes[27]]) as u32 & 0x3FFF;
                let h = u16::from_le_bytes([bytes[28], bytes[29]]) as u32 & 0x3FFF;
                return (Some(w), Some(h));
            }
            // Lossless: a signature byte, then width-1 and height-1 in 14 bits each.
            b"VP8L" if bytes[20] == 0x2F => {
                let bits = u32::from_le_bytes([bytes[21], bytes[22], bytes[23], bytes[24]]);
                return (Some((bits & 0x3FFF) + 1), Some(((bits >> 14) & 0x3FFF) + 1));
            }
            _ => {}
        }
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
        std::fs::create_dir(&home).unwrap();
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

    fn scratch_app() -> (Arc<App>, PathBuf) {
        let home = std::env::temp_dir().join(format!("beans-files-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&home).unwrap();
        (App::load(crate::config::Config { home: home.clone(), port: 0 }).unwrap(), home)
    }

    fn outgoing(home: &Path, file: &str, bytes: &[u8], id: &str) -> OutgoingFile {
        let path = home.join(file);
        std::fs::write(&path, bytes).unwrap();
        OutgoingFile { path: path.to_string_lossy().into_owned(), id: Some(id.into()), name: Some(file.into()), mime: None, width: None, height: None }
    }

    fn attachment(id: &str, name: &str, size: u64) -> Attachment {
        Attachment { id: id.into(), name: name.into(), mime: "text/plain".into(), size, width: None, height: None }
    }

    #[test]
    fn ids_that_are_not_plain_names_are_refused() {
        for good in ["att-0a1b2c3d4e5f", "image-1", "5b3c7d0e-1111-4222-8333-944445555666", "a_b"] {
            assert!(valid_id(good), "{good}");
        }
        for bad in ["", "../machine.json", "..", ".", "a/b", "a\\b", "x.inline.json", "att-a\n", "att-\u{e9}", &"a".repeat(65)] {
            assert!(!valid_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_synced_id_cannot_reach_files_outside_the_store() {
        let (app, home) = scratch_app();
        std::fs::create_dir_all(app.config.files_dir()).unwrap();
        std::fs::write(home.join("machine.json"), b"{\"secret\":true}").unwrap();
        let evil = attachment("../machine.json", "machine.json", 0);
        assert!(local_path(&app, &evil.id).is_err());
        assert!(!is_local(&app, &evil.id));
        let workdir = home.join("work");
        assert_eq!(materialize(&app, &evil, &workdir), None);
        assert!(!workdir.join("attachments").exists(), "nothing was copied into the workspace");
        assert!(write_local(&app, &evil.id, b"x").is_err());
        assert_eq!(std::fs::read(home.join("machine.json")).unwrap(), b"{\"secret\":true}");
        #[cfg(feature = "runner")]
        assert!(inline_image(&app, &Attachment { mime: "image/png".into(), ..evil.clone() }).is_err());
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let error = runtime.block_on(ensure_local(&app, &evil)).unwrap_err();
        assert!(error.to_string().contains("not valid"), "{error}");
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn resending_an_id_replaces_every_copy_made_from_the_old_bytes() {
        let (app, home) = scratch_app();
        let workdir = home.join("work");
        let first = store(&app, &outgoing(&home, "notes.txt", b"first", "att-same")).unwrap();
        let copied = materialize(&app, &first, &workdir).unwrap();
        assert_eq!(std::fs::read(&copied).unwrap(), b"first");
        let inline = app.config.files_dir().join("att-same.inline.json");
        std::fs::write(&inline, b"{\"Err\":\"made from the first bytes\"}").unwrap();

        let second = store(&app, &outgoing(&home, "notes.txt", b"second, longer", "att-same")).unwrap();
        assert_eq!((second.id.as_str(), second.size), ("att-same", 14));
        #[cfg(feature = "runner")]
        assert!(!inline.exists(), "the prepared image of the old bytes is dropped");
        assert_eq!(std::fs::read(materialize(&app, &second, &workdir).unwrap()).unwrap(), b"second, longer");
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn a_batch_with_a_repeated_id_is_refused_before_anything_is_stored() {
        let (app, home) = scratch_app();
        let files = [outgoing(&home, "a.txt", b"a", "att-dup"), outgoing(&home, "b.txt", b"bb", "att-dup")];
        let error = store_many(&app, &files).unwrap_err();
        assert!(error.to_string().contains("share the id att-dup"), "{error}");
        assert!(!is_local(&app, "att-dup"));
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn stored_size_is_what_was_read_and_bytes_of_another_size_are_not_used() {
        let (app, home) = scratch_app();
        let stored = store(&app, &outgoing(&home, "n.txt", b"12345", "att-size")).unwrap();
        assert_eq!(stored.size, 5);
        // Bytes on disk that do not match what the sender declared are never handed to a bot.
        let declared_other = attachment("att-size", "n.txt", 9);
        assert_eq!(materialize(&app, &declared_other, &home.join("work")), None);
        // Older senders and mocks declare 0: the bytes are used as they are.
        assert!(materialize(&app, &attachment("att-size", "n.txt", 0), &home.join("work")).is_some());
        drop(app);
        let _ = std::fs::remove_dir_all(home);
    }

    #[test]
    fn workspace_names_are_single_safe_and_short() {
        assert_eq!(safe_name("line\nbreak\r.txt"), "line_break_.txt");
        assert_eq!(safe_name("  .hidden  "), "hidden");
        let long = format!("{}.pdf", "\u{e9}".repeat(300));
        let short = safe_name(&long);
        assert!(short.len() <= MAX_NAME_BYTES && short.ends_with(".pdf"), "{} bytes", short.len());
        assert_eq!(safe_name(&"x".repeat(400)).len(), MAX_NAME_BYTES);
    }

    #[test]
    fn mime_needs_a_real_extension() {
        assert_eq!(mime_for("png"), "application/octet-stream");
        assert_eq!(mime_for("archive.tar.ZIP"), "application/zip");
        assert_eq!(mime_for("photo."), "application/octet-stream");
    }

    #[test]
    fn header_sizes_for_every_supported_format() {
        let mut gif = b"GIF89a".to_vec();
        gif.extend_from_slice(&[0x20, 0x01, 0x10, 0x00]);
        assert_eq!(image_size(&gif), (Some(288), Some(16)));
        let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00, 0xFF, 0xC0, 0x00, 0x11, 0x08, 0x00, 0x30, 0x00, 0x40, 0x03, 0, 0];
        assert_eq!(image_size(&jpeg), (Some(64), Some(48)));
        let riff = |chunk: &[u8; 4], payload: &[u8]| {
            let mut bytes = b"RIFF\0\0\0\0WEBP".to_vec();
            bytes.extend_from_slice(chunk);
            bytes.extend_from_slice(&[0; 4]);
            bytes.extend_from_slice(payload);
            bytes.resize(32, 0);
            bytes
        };
        let mut vp8x = [0u8; 10];
        vp8x[4..7].copy_from_slice(&[99, 0, 0]);
        vp8x[7..10].copy_from_slice(&[49, 0, 0]);
        assert_eq!(image_size(&riff(b"VP8X", &vp8x)), (Some(100), Some(50)));
        // Lossy: frame tag (3 bytes), start code, then 14-bit width and height.
        let vp8 = [0, 0, 0, 0x9D, 0x01, 0x2A, 0x40, 0x01, 0xF0, 0x00];
        assert_eq!(image_size(&riff(b"VP8 ", &vp8)), (Some(320), Some(240)));
        // Lossless: signature, then (width-1) | (height-1) << 14.
        let bits: u32 = 199 | (99 << 14);
        let mut vp8l = vec![0x2F];
        vp8l.extend_from_slice(&bits.to_le_bytes());
        assert_eq!(image_size(&riff(b"VP8L", &vp8l)), (Some(200), Some(100)));
    }

    #[test]
    fn text_body_without_attachments_still_parses() {
        let body: crate::model::Body = serde_json::from_str(r#"{"kind":"text","text":"hi"}"#).unwrap();
        assert_eq!(body, crate::model::Body::text("hi"));
        assert_eq!(serde_json::to_string(&body).unwrap(), r#"{"kind":"text","text":"hi"}"#);
    }
}
