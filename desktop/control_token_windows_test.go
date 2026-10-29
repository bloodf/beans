//go:build windows

package main

import (
	"os"
	"path/filepath"
	"runtime"
	"syscall"
	"testing"
	"unsafe"
)

// prepareWindowsDesktopTestControlHome applies the private control DACL to a
// disposable home and its format marker. Production never repairs existing
// objects; fixtures must open WRITE_DAC on the inode (CreateFile) because
// GitHub-hosted runners often refuse ReOpenFile(WRITE_DAC) on TempDir handles.
func prepareWindowsDesktopTestControlHome(t *testing.T, home string) {
	t.Helper()
	for _, path := range []string{home, filepath.Join(home, "format.json")} {
		if _, err := os.Stat(path); err != nil {
			t.Fatal(err)
		}
		if err := secureDesktopControlPath(path); err != nil {
			if err == syscall.ERROR_ACCESS_DENIED {
				t.Skipf("private Windows fixture unavailable: %v", err)
			}
			t.Fatal("private Windows fixture", err)
		}
	}
}

func secureDesktopControlPath(path string) error {
	token, err := syscall.OpenCurrentProcessToken()
	if err != nil {
		return err
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil {
		return err
	}
	sid, err := user.User.Sid.String()
	if err != nil {
		return err
	}
	text, err := syscall.UTF16PtrFromString("D:P(A;;FA;;;" + sid + ")")
	if err != nil {
		return err
	}
	var descriptor uintptr
	ok, _, callErr := desktopConvertSecurity.Call(uintptr(unsafe.Pointer(text)), 1, uintptr(unsafe.Pointer(&descriptor)), 0)
	runtime.KeepAlive(text)
	if ok == 0 {
		return callErr
	}
	defer desktopFreeSecurity.Call(descriptor)
	var present, defaulted uint32
	var acl uintptr
	ok, _, callErr = desktopReadDACL.Call(descriptor, uintptr(unsafe.Pointer(&present)), uintptr(unsafe.Pointer(&acl)), uintptr(unsafe.Pointer(&defaulted)))
	if ok == 0 {
		return callErr
	}
	if present == 0 || acl == 0 {
		return syscall.EINVAL
	}
	pathPtr, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	// WRITE_DAC | READ_CONTROL, with BACKUP_SEMANTICS so directories reopen.
	handle, err := syscall.CreateFile(pathPtr, 0x00040000|0x00020000,
		syscall.FILE_SHARE_READ|syscall.FILE_SHARE_WRITE|syscall.FILE_SHARE_DELETE,
		nil, syscall.OPEN_EXISTING, syscall.FILE_FLAG_BACKUP_SEMANTICS, 0)
	runtime.KeepAlive(pathPtr)
	if err != nil {
		return err
	}
	defer syscall.CloseHandle(handle)
	status, _, _ := desktopSetSecurity.Call(uintptr(handle), 1, 0x80000004, 0, 0, acl, 0)
	runtime.KeepAlive(user)
	if status != 0 {
		return syscall.Errno(status)
	}
	return nil
}
