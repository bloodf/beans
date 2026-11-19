//! One file out of a zip archive (a Python wheel), read from memory: the ONNX Runtime library
//! inside its official wheel (`runtime_download`). Only what wheels use is read: stored and
//! deflated members, no zip64, no encryption. The caller has already checked the archive's
//! size and SHA-256 against its pin and checks the member's against its own, so this only
//! has to refuse, never to trust.

use std::io::Read;

/// Signatures of the records read.
const END_OF_DIRECTORY: u32 = 0x0605_4b50;
const DIRECTORY_ENTRY: u32 = 0x0201_4b50;
const LOCAL_HEADER: u32 = 0x0403_4b50;
/// The end record is 22 bytes, after which a comment of at most 65,535 bytes may follow.
const END_RECORD: usize = 22;
const MAX_COMMENT: usize = 65_535;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// The member of `zip` named `name`, unpacked, if it is there and unpacks to exactly the size
/// its directory entry gives, at most `max` bytes. `Err` says why not, for logs and tests.
pub fn extract_member(zip: &[u8], name: &str, max: u64) -> Result<Vec<u8>, String> {
    let last = zip.len().checked_sub(END_RECORD).ok_or("not a zip archive")?;
    let end = (last.saturating_sub(MAX_COMMENT)..=last).rev().find(|&at| u32_at(zip, at) == Some(END_OF_DIRECTORY)).ok_or("not a zip archive")?;
    let count = u16_at(zip, end + 10).ok_or("not a zip archive")?;
    let mut at = u32_at(zip, end + 16).ok_or("not a zip archive")? as usize;
    for _ in 0..count {
        if at.checked_add(46).is_none_or(|header| header > end) || u32_at(zip, at) != Some(DIRECTORY_ENTRY) {
            return Err("a damaged zip directory".into());
        }
        let field = |offset: usize| u16_at(zip, at + offset).map(usize::from).ok_or("a damaged zip directory");
        let method = field(10)?;
        let compressed = u32_at(zip, at + 20).ok_or("a damaged zip directory")? as usize;
        let size = u32_at(zip, at + 24).ok_or("a damaged zip directory")?;
        let (name_length, extra_length, comment_length) = (field(28)?, field(30)?, field(32)?);
        let local = u32_at(zip, at + 42).ok_or("a damaged zip directory")? as usize;
        let entry_name = zip.get(at + 46..at + 46 + name_length).ok_or("a damaged zip directory")?;
        at += 46 + name_length + extra_length + comment_length;
        if entry_name != name.as_bytes() {
            continue;
        }
        if u64::from(size) > max {
            return Err(format!("{name} is {size} bytes, more than {max}"));
        }
        if local.checked_add(30).is_none_or(|header| header > end) || u32_at(zip, local) != Some(LOCAL_HEADER) {
            return Err("a damaged zip entry".into());
        }
        let start = local + 30 + usize::from(u16_at(zip, local + 26).unwrap_or(0)) + usize::from(u16_at(zip, local + 28).unwrap_or(0));
        let data = start.checked_add(compressed).filter(|&stop| stop <= end).and_then(|stop| zip.get(start..stop)).ok_or_else(|| format!("{name} runs past the archive"))?;
        let out = match method {
            0 => data.to_vec(),
            8 => {
                // Never more than one byte past the size the directory gives: a bomb stops there.
                let mut out = Vec::with_capacity(size as usize);
                flate2::read::DeflateDecoder::new(data).take(u64::from(size) + 1).read_to_end(&mut out).map_err(|error| format!("{name} does not inflate: {error}"))?;
                out
            }
            other => return Err(format!("{name} uses zip method {other}")),
        };
        if out.len() as u64 != u64::from(size) {
            return Err(format!("{name} unpacked to {} bytes, not {size}", out.len()));
        }
        return Ok(out);
    }
    Err(format!("{name} is not in the archive"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// A zip of `files` as a wheel holds them: the first stored, the rest deflated. `method`
    /// overrides the method the second file's entries record.
    pub(crate) fn zip(files: &[(&str, &[u8])], method: Option<u16>) -> Vec<u8> {
        let (mut locals, mut directory) = (Vec::new(), Vec::new());
        for (index, (name, data)) in files.iter().enumerate() {
            let stored = if index == 0 { 0u16 } else { 8 };
            let body = if stored == 0 {
                data.to_vec()
            } else {
                let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                encoder.write_all(data).unwrap();
                encoder.finish().unwrap()
            };
            let recorded = if index == 1 { method.unwrap_or(stored) } else { stored };
            let offset = locals.len() as u32;
            locals.extend(LOCAL_HEADER.to_le_bytes());
            locals.extend([0u8; 4]);
            locals.extend(recorded.to_le_bytes());
            locals.extend([0u8; 8]);
            locals.extend((body.len() as u32).to_le_bytes());
            locals.extend((data.len() as u32).to_le_bytes());
            locals.extend((name.len() as u16).to_le_bytes());
            locals.extend([0u8; 2]);
            locals.extend(name.as_bytes());
            locals.extend(&body);
            directory.extend(DIRECTORY_ENTRY.to_le_bytes());
            directory.extend([0u8; 6]);
            directory.extend(recorded.to_le_bytes());
            directory.extend([0u8; 8]);
            directory.extend((body.len() as u32).to_le_bytes());
            directory.extend((data.len() as u32).to_le_bytes());
            directory.extend((name.len() as u16).to_le_bytes());
            directory.extend([0u8; 12]);
            directory.extend(offset.to_le_bytes());
            directory.extend(name.as_bytes());
        }
        let mut zip = locals.clone();
        zip.extend(&directory);
        zip.extend(END_OF_DIRECTORY.to_le_bytes());
        zip.extend([0u8; 4]);
        zip.extend((files.len() as u16).to_le_bytes());
        zip.extend((files.len() as u16).to_le_bytes());
        zip.extend((directory.len() as u32).to_le_bytes());
        zip.extend((locals.len() as u32).to_le_bytes());
        zip.extend([0u8; 2]);
        zip
    }

    const LIBRARY: &[u8] = b"\x7fELF library bytes that repeat, library bytes that repeat, library bytes";

    #[test]
    fn a_member_unpacks_whether_stored_or_deflated() {
        let archive = zip(&[("onnxruntime/__init__.py", b"# python"), ("onnxruntime/capi/libonnxruntime.so", LIBRARY)], None);
        assert_eq!(extract_member(&archive, "onnxruntime/capi/libonnxruntime.so", 1 << 20).unwrap(), LIBRARY);
        assert_eq!(extract_member(&archive, "onnxruntime/__init__.py", 1 << 20).unwrap(), b"# python");
        // A comment after the end record, as some tools write.
        let mut commented = archive.clone();
        let comment = b"a comment";
        let length = commented.len();
        commented[length - 2..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
        commented.extend(comment);
        assert_eq!(extract_member(&commented, "onnxruntime/capi/libonnxruntime.so", 1 << 20).unwrap(), LIBRARY);
    }

    #[test]
    fn anything_else_is_refused_never_unpacked_short() {
        let files: &[(&str, &[u8])] = &[("a.py", b"# python"), ("lib.so", LIBRARY)];
        let good = zip(files, None);
        assert_eq!(extract_member(&good, "missing", 1 << 20).unwrap_err(), "missing is not in the archive");
        assert_eq!(extract_member(&good, "lib.s", 1 << 20).unwrap_err(), "lib.s is not in the archive", "names match whole");
        assert!(extract_member(&zip(files, Some(12)), "lib.so", 1 << 20).unwrap_err().contains("zip method 12"));
        assert!(extract_member(&good, "lib.so", 10).unwrap_err().contains("more than 10"), "over the caller's limit");
        let directory = good.windows(4).position(|w| w == DIRECTORY_ENTRY.to_le_bytes()).unwrap();
        // The directory says the stored file is shorter than it is.
        let mut lying = good.clone();
        lying[directory + 24..directory + 28].copy_from_slice(&5u32.to_le_bytes());
        assert_eq!(extract_member(&lying, "a.py", 1 << 20).unwrap_err(), "a.py unpacked to 8 bytes, not 5");
        // The deflated file claims to be smaller than it inflates to: stopped at one byte past.
        let second = directory + 46 + 4;
        let mut bomb = good.clone();
        bomb[second + 24..second + 28].copy_from_slice(&3u32.to_le_bytes());
        assert_eq!(extract_member(&bomb, "lib.so", 1 << 20).unwrap_err(), "lib.so unpacked to 4 bytes, not 3");
        // Damaged bytes inside the deflated data.
        let mut damaged = good.clone();
        let data = good.windows(6).position(|w| w == b"lib.so").unwrap() + 6;
        for byte in &mut damaged[data..data + 8] {
            *byte = 0xff;
        }
        assert!(extract_member(&damaged, "lib.so", 1 << 20).is_err());
        // Cut short, pointing past itself, or not a zip at all.
        assert!(extract_member(&good[..good.len() - 30], "lib.so", 1 << 20).is_err());
        let mut astray = good.clone();
        let length = astray.len();
        astray[length - 6..length - 2].copy_from_slice(&(good.len() as u32 * 2).to_le_bytes());
        assert_eq!(extract_member(&astray, "lib.so", 1 << 20).unwrap_err(), "a damaged zip directory");
        let mut wild = good.clone();
        wild[directory + 42..directory + 46].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(extract_member(&wild, "a.py", 1 << 20).unwrap_err(), "a damaged zip entry");
        for nothing in [&b""[..], b"PK", b"not a zip archive, just some bytes that go on"] {
            assert_eq!(extract_member(nothing, "x", 1).unwrap_err(), "not a zip archive");
        }
        assert_eq!(extract_member(&zip(&[], None), "x", 1).unwrap_err(), "x is not in the archive");
    }
}
