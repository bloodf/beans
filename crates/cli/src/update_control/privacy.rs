//! Conservative Windows control-object admission. No existing security descriptor is changed.

fn valid_sid(sid: &[u8]) -> bool {
    sid.len() >= 8 && sid[0] == 1 && sid[1] <= 15 && sid.len() == 8 + 4 * usize::from(sid[1])
}

fn admits(owner: &[u8], user: &[u8], acl: &[u8], private: bool) -> bool {
    const SYSTEM: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0];
    let trusted = |sid: &[u8]| sid == user || sid == SYSTEM;
    if !valid_sid(owner)
        || !valid_sid(user)
        || !trusted(owner)
        || acl.len() < 8
        || !matches!(acl[0], 2 | 4)
        || usize::from(u16::from_le_bytes([acl[2], acl[3]])) != acl.len()
    {
        return false;
    }
    // Replacement, data writes, ownership and ACL changes; generic rights count too.
    let mut unsafe_rights = 0x000D_0156 | 0x5000_0000 | 0x0300_0000;
    if private {
        unsafe_rights |= 0x0000_0029 | 0xA000_0000;
    }
    let mut position = 8;
    for _ in 0..u16::from_le_bytes([acl[4], acl[5]]) {
        let Some(header) = acl.get(position..position + 4) else {
            return false;
        };
        let size = usize::from(u16::from_le_bytes([header[2], header[3]]));
        let Some(ace) = acl.get(position..position + size) else {
            return false;
        };
        if size < 16 || ace[0] > 1 || ace[1] & !0x1F != 0 || !valid_sid(&ace[8..]) {
            return false;
        }
        let mask = u32::from_le_bytes(ace[4..8].try_into().expect("validated ACE header"));
        if mask & !0xF31F_01FF != 0 {
            return false;
        }
        // INHERIT_ONLY is not effective on this object. Inherited effective allows
        // remain subject to admission; a deny never excuses an unsafe allow.
        if ace[0] == 0 && ace[1] & 8 == 0 && !trusted(&ace[8..]) && mask & unsafe_rights != 0 {
            return false;
        }
        position += size;
    }
    true // ACL allocation may include unused space after its declared ACEs.
}

#[cfg(windows)]
mod native {
    use std::fs::{File, OpenOptions};
    use std::io;
    use std::os::windows::{
        ffi::OsStringExt,
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    };
    use std::path::PathBuf;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INSUFFICIENT_BUFFER, GetLastError, HANDLE, LocalFree},
        Security::{
            Authorization::{GetSecurityInfo, SE_FILE_OBJECT},
            DACL_SECURITY_INFORMATION, GetSecurityDescriptorLength, GetTokenInformation,
            IsValidSecurityDescriptor, OWNER_SECURITY_INFORMATION, TOKEN_QUERY, TOKEN_USER,
            TokenUser,
        },
        Storage::FileSystem::{
            FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
            GetFinalPathNameByHandleW,
        },
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
    struct Descriptor(*mut std::ffi::c_void);
    impl Drop for Descriptor {
        fn drop(&mut self) {
            unsafe {
                LocalFree(self.0);
            }
        }
    }
    fn refused() -> io::Error {
        io::Error::from(io::ErrorKind::PermissionDenied)
    }

    // Read only bounded pieces of the OS-owned descriptor/token buffer.
    unsafe fn sid<'a>(pointer: *const u8, base: *const u8, size: usize) -> io::Result<&'a [u8]> {
        let offset = (pointer as usize)
            .checked_sub(base as usize)
            .ok_or_else(refused)?;
        if offset > size || size - offset < 8 {
            return Err(refused());
        }
        let header = unsafe { std::slice::from_raw_parts(pointer, 8) };
        let length = 8 + 4 * usize::from(header[1]);
        if header[0] != 1 || header[1] > 15 || length > size - offset {
            return Err(refused());
        }
        Ok(unsafe { std::slice::from_raw_parts(pointer, length) })
    }

    fn validate(file: &File, private: bool) -> io::Result<()> {
        if file.metadata()?.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err(refused());
        }
        let mut token = std::ptr::null_mut();
        if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
            return Err(io::Error::last_os_error());
        }
        let token = Token(token);
        let mut needed = 0;
        if unsafe { GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut needed) }
            != 0
            || unsafe { GetLastError() } != ERROR_INSUFFICIENT_BUFFER
            || !(16..=4096).contains(&needed)
        {
            return Err(refused());
        }
        let mut buffer = vec![0usize; (needed as usize).div_ceil(std::mem::size_of::<usize>())];
        if unsafe {
            GetTokenInformation(
                token.0,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                needed,
                &mut needed,
            )
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let user = unsafe {
            sid(
                user.User.Sid.cast(),
                buffer.as_ptr().cast(),
                buffer.len() * std::mem::size_of::<usize>(),
            )?
        };
        let mut owner = std::ptr::null_mut();
        let mut acl = std::ptr::null_mut();
        let mut descriptor = std::ptr::null_mut();
        let status = unsafe {
            GetSecurityInfo(
                file.as_raw_handle(),
                SE_FILE_OBJECT,
                OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                &mut owner,
                std::ptr::null_mut(),
                &mut acl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let descriptor = Descriptor(descriptor);
        if descriptor.0.is_null() || unsafe { IsValidSecurityDescriptor(descriptor.0) } == 0 {
            return Err(refused());
        }
        let size = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
        let owner = unsafe { sid(owner.cast(), descriptor.0.cast(), size)? };
        let offset = (acl as usize)
            .checked_sub(descriptor.0 as usize)
            .ok_or_else(refused)?;
        if offset > size || size - offset < 8 {
            return Err(refused());
        }
        let header = unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), 8) };
        let length = usize::from(u16::from_le_bytes([header[2], header[3]]));
        if length < 8 || length > size - offset {
            return Err(refused());
        }
        let acl = unsafe { std::slice::from_raw_parts(acl.cast::<u8>(), length) };
        if !super::admits(owner, user, acl, private) {
            return Err(refused());
        }
        Ok(())
    }

    fn final_path(file: &File) -> io::Result<PathBuf> {
        let needed =
            unsafe { GetFinalPathNameByHandleW(file.as_raw_handle(), std::ptr::null_mut(), 0, 0) };
        if needed == 0 {
            return Err(io::Error::last_os_error());
        }
        if needed > 32768 {
            return Err(refused());
        }
        let mut text = vec![0u16; needed as usize];
        let written = unsafe {
            GetFinalPathNameByHandleW(file.as_raw_handle(), text.as_mut_ptr(), needed, 0)
        };
        if written == 0 {
            return Err(io::Error::last_os_error());
        }
        if written >= needed {
            return Err(refused());
        }
        Ok(PathBuf::from(std::ffi::OsString::from_wide(
            &text[..written as usize],
        )))
    }

    pub(in crate::update_control) fn validate_token(file: &File) -> io::Result<File> {
        validate(file, true)?;
        // Resolve the already-opened token's actual parent, not an unchecked
        // configured pathname. Hold it through the caller's secret read.
        let token_path = final_path(file)?;
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT)
            .open(token_path.parent().ok_or_else(refused)?)?;
        if !parent.metadata()?.is_dir() {
            return Err(refused());
        }
        validate(&parent, false)?;
        if final_path(file)?.parent() != Some(final_path(&parent)?.as_path()) {
            return Err(refused());
        }
        Ok(parent)
    }

    #[cfg(test)]
    #[test]
    fn existing_windows_token_and_directory_are_refused_without_repair() {
        use windows_sys::Win32::Security::{
            Authorization::{
                ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
                SetSecurityInfo,
            },
            GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION,
        };
        let mut token = std::ptr::null_mut();
        assert_ne!(
            unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) },
            0
        );
        let token = Token(token);
        let mut buffer = [0usize; 128];
        let mut needed = 0;
        assert_ne!(
            unsafe {
                GetTokenInformation(
                    token.0,
                    TokenUser,
                    buffer.as_mut_ptr().cast(),
                    std::mem::size_of_val(&buffer) as u32,
                    &mut needed,
                )
            },
            0
        );
        let user = unsafe { &*buffer.as_ptr().cast::<TOKEN_USER>() };
        let mut text = std::ptr::null_mut();
        assert_ne!(
            unsafe { ConvertSidToStringSidW(user.User.Sid, &mut text) },
            0
        );
        let length = (0..256)
            .find(|&i| unsafe { *text.add(i) } == 0)
            .expect("bounded user SID");
        let user_text =
            String::from_utf16(unsafe { std::slice::from_raw_parts(text, length) }).unwrap();
        unsafe {
            LocalFree(text.cast());
        }
        let set_acl = |file: &File, extra: &str| {
            let text: Vec<_> = format!("D:P(A;;FA;;;{user_text}){extra}")
                .encode_utf16()
                .chain([0])
                .collect();
            let mut descriptor = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    ConvertStringSecurityDescriptorToSecurityDescriptorW(
                        text.as_ptr(),
                        1,
                        &mut descriptor,
                        std::ptr::null_mut(),
                    )
                },
                0
            );
            let descriptor = Descriptor(descriptor);
            let mut present = 0;
            let mut defaulted = 0;
            let mut acl = std::ptr::null_mut();
            assert_ne!(
                unsafe {
                    GetSecurityDescriptorDacl(descriptor.0, &mut present, &mut acl, &mut defaulted)
                },
                0
            );
            assert_eq!(present, 1);
            assert_eq!(
                unsafe {
                    SetSecurityInfo(
                        file.as_raw_handle(),
                        SE_FILE_OBJECT,
                        DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        acl,
                        std::ptr::null(),
                    )
                },
                0
            );
        };
        let snapshot = |file: &File| {
            let mut descriptor = std::ptr::null_mut();
            assert_eq!(
                unsafe {
                    GetSecurityInfo(
                        file.as_raw_handle(),
                        SE_FILE_OBJECT,
                        OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                        &mut descriptor,
                    )
                },
                0
            );
            let descriptor = Descriptor(descriptor);
            let size = unsafe { GetSecurityDescriptorLength(descriptor.0) } as usize;
            unsafe { std::slice::from_raw_parts(descriptor.0.cast::<u8>(), size) }.to_vec()
        };
        let home = tempfile::tempdir().unwrap();
        let directory = OpenOptions::new()
            .read(true)
            .access_mode(0x60001)
            .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
            .open(home.path())
            .unwrap();
        set_acl(&directory, "");
        let path = home.path().join("token");
        let secret = "x".repeat(32);
        std::fs::write(&path, &secret).unwrap();
        let file = OpenOptions::new()
            .read(true)
            .access_mode(0x60001)
            .open(&path)
            .unwrap();
        set_acl(&file, "");
        assert_eq!(crate::update_control::read_token(&path).unwrap(), secret);
        for (target, grant) in [
            (&file, "(A;;FR;;;WD)"),
            (&file, "(A;;FR;;;BA)"),
            (&directory, "(A;;GW;;;WD)"),
        ] {
            set_acl(target, grant);
            let before = snapshot(target);
            assert!(crate::update_control::read_token(&path).is_err());
            assert_eq!(std::fs::read_to_string(&path).unwrap(), secret);
            assert_eq!(
                snapshot(target),
                before,
                "existing owner/DACL is never repaired"
            );
            set_acl(target, "");
        }
    }
}

#[cfg(windows)]
pub(super) use native::validate_token;

#[cfg(test)]
mod tests {
    use super::*;
    const USER: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0];
    const OTHER: &[u8] = &[1, 1, 0, 0, 0, 0, 0, 5, 22, 0, 0, 0];
    fn acl(entries: &[(u8, u8, u32, &[u8])]) -> Vec<u8> {
        let mut data = vec![2, 0, 0, 0, entries.len() as u8, 0, 0, 0];
        for &(kind, flags, mask, sid) in entries {
            data.extend([kind, flags]);
            data.extend(((8 + sid.len()) as u16).to_le_bytes());
            data.extend(mask.to_le_bytes());
            data.extend(sid);
        }
        let size = (data.len() as u16).to_le_bytes();
        data[2..4].copy_from_slice(&size);
        data
    }
    #[test]
    fn windows_privacy_rejects_foreign_owner_and_effective_grants() {
        let own = (0, 0, 0x1F01FF, USER);
        assert!(admits(USER, USER, &acl(&[own]), true));
        assert!(!admits(OTHER, USER, &acl(&[own]), true));
        let admins = &[1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0];
        assert!(!admits(USER, USER, &acl(&[own, (0, 0, 1, admins)]), true));
        for mask in [1, 2, 4, 0x10000, 0x40000, 0x80000, 0x80000000, 0x40000000] {
            for flags in [0, 0x10] {
                assert!(!admits(
                    USER,
                    USER,
                    &acl(&[own, (0, flags, mask, OTHER)]),
                    true
                ));
            }
        }
        assert!(admits(USER, USER, &acl(&[own, (0, 8, 1, OTHER)]), true));
        assert!(!admits(
            USER,
            USER,
            &acl(&[own, (1, 0, 1, OTHER), (0, 0, 1, OTHER)]),
            true
        ));
        for mask in [2, 4, 0x40, 0x10000, 0x40000, 0x80000] {
            assert!(!admits(
                USER,
                USER,
                &acl(&[own, (0, 0, mask, OTHER)]),
                false
            ));
        }
    }
    #[test]
    fn windows_privacy_rejects_null_truncated_or_unsupported_acl_evidence() {
        assert!(!admits(USER, USER, &[], true));
        let own = (0, 0, 0x1F01FF, USER);
        let good = acl(&[own]);
        for length in 0..good.len() {
            assert!(!admits(USER, USER, &good[..length], true));
        }
        for kind in [2, 5, 9, 255] {
            assert!(!admits(USER, USER, &acl(&[(kind, 0, 1, USER)]), true));
        }
        assert!(!admits(USER, USER, &acl(&[(0, 0, 0x00000200, USER)]), true));
        let mut bad = good;
        bad[9] = 0x80;
        assert!(!admits(USER, USER, &bad, true));
    }
}
