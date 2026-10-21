//! Handle-relative local token opens, using the existing proposal publishing pattern.
use std::fs::File;
use std::os::windows::io::{AsRawHandle, FromRawHandle};
use windows_sys::Wdk::Foundation::OBJECT_ATTRIBUTES;
use windows_sys::Wdk::Storage::FileSystem::{NtCreateFile, FILE_CREATE, FILE_OPEN, FILE_NON_DIRECTORY_FILE, FILE_DIRECTORY_FILE, FILE_OPEN_REPARSE_POINT, FILE_SYNCHRONOUS_IO_NONALERT};
use windows_sys::Win32::Foundation::{OBJ_CASE_INSENSITIVE, OBJ_DONT_REPARSE, STATUS_OBJECT_NAME_NOT_FOUND, STATUS_OBJECT_PATH_NOT_FOUND, UNICODE_STRING};
use windows_sys::Win32::Storage::FileSystem::{DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_SHARE_DELETE, SYNCHRONIZE};
use windows_sys::Win32::System::IO::IO_STATUS_BLOCK;

pub(super) fn open(parent: &File, name: &str, create: bool, directory: bool) -> std::io::Result<File> {
    let mut name: Vec<u16> = name.encode_utf16().collect();
    let bytes = u16::try_from(name.len() * 2).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    let unicode = UNICODE_STRING { Length: bytes, MaximumLength: bytes, Buffer: name.as_mut_ptr() };
    let attributes = OBJECT_ATTRIBUTES {
        Length: std::mem::size_of::<OBJECT_ATTRIBUTES>() as u32,
        RootDirectory: parent.as_raw_handle(), ObjectName: &unicode,
        Attributes: OBJ_CASE_INSENSITIVE | OBJ_DONT_REPARSE,
        SecurityDescriptor: std::ptr::null(), SecurityQualityOfService: std::ptr::null(),
    };
    let mut io: IO_STATUS_BLOCK = unsafe { std::mem::zeroed() };
    let mut handle = std::ptr::null_mut();
    let access = if create { 0xC00C0000 | (if directory { 0 } else { DELETE }) | SYNCHRONIZE } else { 0x80000000 | SYNCHRONIZE };
    let status = unsafe { NtCreateFile(&mut handle, access, &attributes, &mut io,
        std::ptr::null(), 0, if create && !directory { 0 } else if directory { FILE_SHARE_READ | FILE_SHARE_WRITE } else { FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE },
        if create { FILE_CREATE } else { FILE_OPEN },
        (if directory { FILE_DIRECTORY_FILE } else { FILE_NON_DIRECTORY_FILE }) | FILE_OPEN_REPARSE_POINT | FILE_SYNCHRONOUS_IO_NONALERT,
        std::ptr::null(), 0) };
    if status == STATUS_OBJECT_NAME_NOT_FOUND || status == STATUS_OBJECT_PATH_NOT_FOUND {
        return Err(std::io::ErrorKind::NotFound.into());
    }
    if status < 0 { return Err(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Local token handle admission refused")); }
    Ok(unsafe { File::from_raw_handle(handle) })
}
