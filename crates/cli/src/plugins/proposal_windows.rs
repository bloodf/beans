//! Handle-relative Windows draft publishing. Never resolve an untrusted child through a path.
use std::io::Write;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::{Component, Path, PathBuf};
use std::{ffi::OsStr, mem, ptr};

use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{
    NtCreateFile, NtSetInformationFile, FileRenameInformation, FILE_CREATE, FILE_DIRECTORY_FILE,
    FILE_NON_DIRECTORY_FILE, FILE_OPEN, FILE_OPEN_IF, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT,
};
use windows_sys::Win32::Foundation::{
    HANDLE, OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, STATUS_OBJECT_NAME_COLLISION,
    STATUS_OBJECT_NAME_NOT_FOUND, STATUS_OBJECT_PATH_NOT_FOUND, STATUS_REPARSE_POINT_ENCOUNTERED, UNICODE_STRING,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandleEx, FileAttributeTagInfo, DELETE,
    FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_REPARSE_POINT, FILE_ATTRIBUTE_TAG_INFO,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_LIST_DIRECTORY,
    FILE_READ_ATTRIBUTES, FILE_RENAME_INFO, FILE_SHARE_DELETE, FILE_SHARE_READ,
    FILE_SHARE_WRITE, FILE_WRITE_DATA, OPEN_EXISTING, SYNCHRONIZE,
};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

fn validate_segment(segment: &str) -> Result<(), String> {
    let stem = segment.split('.').next().unwrap_or("").trim_end_matches(' ');
    let reserved = matches!(stem.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL" | "COM1" | "COM2" | "COM3" | "COM4" | "COM5" |
        "COM6" | "COM7" | "COM8" | "COM9" | "LPT1" | "LPT2" | "LPT3" | "LPT4" |
        "LPT5" | "LPT6" | "LPT7" | "LPT8" | "LPT9");
    if segment.is_empty() || segment == "." || segment == ".." || segment.ends_with([' ', '.'])
        || reserved || segment.chars().any(|c| c.is_control() || matches!(c, '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')) {
        return Err("Proposal path must be relative, without traversal or Windows device names.".into());
    }
    Ok(())
}

fn checked_directory(handle: &OwnedHandle) -> Result<(), String> {
    let mut info = FILE_ATTRIBUTE_TAG_INFO::default();
    let ok = unsafe { GetFileInformationByHandleEx(handle.as_raw_handle(), FileAttributeTagInfo,
        (&mut info as *mut FILE_ATTRIBUTE_TAG_INFO).cast(), mem::size_of_val(&info) as u32) };
    if ok == 0 { return Err(format!("Inspecting proposal directory: {}", std::io::Error::last_os_error())); }
    if info.FileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err("Proposal path crosses a junction or reparse point.".into());
    }
    if info.FileAttributes & FILE_ATTRIBUTE_DIRECTORY == 0 {
        return Err("Proposal parent is not a directory.".into());
    }
    Ok(())
}

// NtCreateFile interprets each name against the already-open directory handle, not against
// a path reconstructed from it. OBJ_DONT_REPARSE also rejects reparses while opening.
fn open_child(parent: &OwnedHandle, name: &OsStr, create: bool, workspace_dir: bool) -> Result<OwnedHandle, String> {
    let mut wide: Vec<u16> = name.encode_wide().collect();
    let bytes = wide.len().checked_mul(2).and_then(|n| u16::try_from(n).ok())
        .ok_or("Proposal path segment is too long.")?;
    let unicode = UNICODE_STRING { Length: bytes, MaximumLength: bytes, Buffer: wide.as_mut_ptr() };
    let attrs = OBJECT_ATTRIBUTES { Length: mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(), ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: ptr::null(), SecurityQualityOfService: ptr::null() };
    let mut status_block: IO_STATUS_BLOCK = unsafe { mem::zeroed() };
    let mut raw: HANDLE = ptr::null_mut();
    let access = if create { FILE_WRITE_DATA | DELETE | SYNCHRONIZE }
        else { FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES | SYNCHRONIZE };
    let options = if create { FILE_NON_DIRECTORY_FILE } else { FILE_DIRECTORY_FILE };
    let mut status = unsafe { NtCreateFile(&mut raw, access, &attrs, &mut status_block, ptr::null(),
        0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
        if create { FILE_CREATE } else { FILE_OPEN },
        options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        ptr::null(), 0) };
    if workspace_dir && (status == STATUS_OBJECT_NAME_NOT_FOUND || status == STATUS_OBJECT_PATH_NOT_FOUND) {
        status = unsafe { NtCreateFile(&mut raw, access, &attrs, &mut status_block, ptr::null(),
            FILE_ATTRIBUTE_DIRECTORY, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            FILE_OPEN_IF, options | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
            ptr::null(), 0) };
    }
    if status < 0 {
        if status == STATUS_REPARSE_POINT_ENCOUNTERED {
            return Err("Proposal path crosses a junction or reparse point.".into());
        }
        if status == STATUS_OBJECT_NAME_COLLISION {
            return Err("Proposal draft already exists; existing content was not changed.".into());
        }
        return Err(format!("{}: NTSTATUS {status:#010x}",
            if create { "Creating draft" } else { "Opening proposal parent" }));
    }
    // SAFETY: successful NtCreateFile transfers a non-null owned handle.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
    if !create { checked_directory(&handle)?; }
    Ok(handle)
}

pub(super) fn save_proposal(workdir: &Path, path: &str, content: &str) -> Result<(), String> {
    if path.is_empty() || Path::new(path).is_absolute() { return Err("Proposal path must stay inside bot workspace.".into()); }
    let parts: Vec<_> = path.split('/').collect();
    for part in &parts { validate_segment(part)?; }
    let absolute = std::path::absolute(workdir).map_err(|e| format!("Opening workspace: {e}"))?;
    let mut components = absolute.components();
    let mut volume = PathBuf::new();
    let Some(Component::Prefix(prefix)) = components.next() else { return Err("Workspace needs an absolute Windows path.".into()); };
    volume.push(prefix.as_os_str());
    if !matches!(components.next(), Some(Component::RootDir)) { return Err("Workspace needs an absolute Windows path.".into()); }
    volume.push("\\");
    let wide: Vec<u16> = volume.as_os_str().encode_wide().chain([0]).collect();
    let raw = unsafe { CreateFileW(wide.as_ptr(), FILE_LIST_DIRECTORY | FILE_READ_ATTRIBUTES,
        FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, ptr::null(), OPEN_EXISTING,
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT, ptr::null_mut()) };
    if raw == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
        return Err(format!("Opening workspace volume: {}", std::io::Error::last_os_error()));
    }
    let mut dir = unsafe { OwnedHandle::from_raw_handle(raw) };
    checked_directory(&dir)?;
    for component in components {
        let Component::Normal(name) = component else { return Err("Workspace path contains traversal.".into()); };
        dir = open_child(&dir, name, false, true)?;
    }
    for part in &parts[..parts.len() - 1] { dir = open_child(&dir, OsStr::new(*part), false, false)?; }

    // A temp in this opened parent stays there even if its old pathname is renamed elsewhere.
    // Rename through the same handle, with ReplaceIfExists=false, is atomic and no-clobber.
    let temp_name = format!(".lorca-propose-{}", uuid::Uuid::new_v4());
    let temporary = open_child(&dir, OsStr::new(&temp_name), true, false)?;
    let mut temporary = std::fs::File::from(temporary);
    let result = (|| -> Result<(), String> {
        temporary.write_all(content.as_bytes()).map_err(|e| format!("Writing draft: {e}"))?;
        temporary.sync_all().map_err(|e| format!("Syncing draft: {e}"))?;
        let name: Vec<u16> = OsStr::new(parts[parts.len() - 1]).encode_wide().collect();
        let offset = mem::offset_of!(FILE_RENAME_INFO, FileName);
        let size = mem::size_of::<FILE_RENAME_INFO>().max(offset + name.len() * 2);
        let mut buffer = vec![0usize; size.div_ceil(mem::size_of::<usize>())];
        let rename = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
        unsafe {
            (*rename).Anonymous.ReplaceIfExists = false;
            (*rename).RootDirectory = dir.as_raw_handle();
            (*rename).FileNameLength = (name.len() * 2) as u32;
            ptr::copy_nonoverlapping(name.as_ptr(), ptr::addr_of_mut!((*rename).FileName).cast(), name.len());
        }
        let mut io: IO_STATUS_BLOCK = unsafe { mem::zeroed() };
        let status = unsafe { NtSetInformationFile(temporary.as_raw_handle(), &mut io,
            rename.cast(), size as u32, FileRenameInformation) };
        if status == STATUS_OBJECT_NAME_COLLISION {
            return Err("Proposal draft already exists; existing content was not changed.".into());
        }
        if status < 0 { return Err(format!("Publishing draft without overwrite: NTSTATUS {status:#010x}")); }
        Ok(())
    })();
    if result.is_err() {
        // Delete by the original directory handle, never by a reconstructed workspace path.
        let mut io: IO_STATUS_BLOCK = unsafe { mem::zeroed() };
        let delete: u8 = 1;
        unsafe { NtSetInformationFile(temporary.as_raw_handle(), &mut io,
            (&delete as *const u8).cast(), 1,
            windows_sys::Wdk::Storage::FileSystem::FileDispositionInformation); }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::validate_segment;

    #[test]
    fn reject_windows_aliases_and_traversal() {
        for bad in ["", ".", "..", "NUL", "con.txt", "Com1.log", "a:", "a\\b", "a.", "a ", "a?b"] {
            assert!(validate_segment(bad).is_err(), "{bad}");
        }
        assert!(validate_segment("draft.md").is_ok());
    }

    #[test]
    fn no_clobber_and_no_junction_traversal() {
        let base = std::env::temp_dir().join(format!("lorca-proposal-{}", uuid::Uuid::new_v4()));
        let workspace = base.join("workspace");
        std::fs::create_dir_all(workspace.join("notes")).unwrap();
        super::save_proposal(&workspace, "notes/draft.md", "one").unwrap();
        assert!(super::save_proposal(&workspace, "notes/draft.md", "two").is_err());
        assert_eq!(std::fs::read_to_string(workspace.join("notes/draft.md")).unwrap(), "one");
        assert!(super::save_proposal(&workspace, "../escape", "x").is_err());
        if std::os::windows::fs::symlink_dir(&base, workspace.join("outside")).is_ok() {
            assert!(super::save_proposal(&workspace, "outside/escape", "x").is_err());
            assert!(!base.join("escape").exists());
        }
        std::fs::remove_dir_all(base).unwrap();
    }
}
