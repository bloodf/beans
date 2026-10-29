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

var desktopSetNamedSecurity = desktopControlSecurity.NewProc("SetNamedSecurityInfoW")

// prepareWindowsDesktopTestControlHome applies the private control DACL to a
// disposable home and its format marker. Production never repairs existing
// objects; fixtures recreate a creator-owned directory and exclusive marker,
// then apply the same private SDDL production uses (TempDir inheritance on
// GitHub-hosted runners refuses WRITE_DAC on the original TempDir inode).
func prepareWindowsDesktopTestControlHome(t *testing.T, home string) {
	t.Helper()
	marker := filepath.Join(home, "format.json")
	data, err := os.ReadFile(marker)
	if err != nil {
		t.Fatal(err)
	}
	aside := home + ".aside"
	if err := os.Rename(home, aside); err != nil {
		failDesktopControlFixture(t, err)
	}
	if err := os.Mkdir(home, 0o700); err != nil {
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	if err := secureDesktopControlPath(home); err != nil {
		_ = os.RemoveAll(home)
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	file, err := os.OpenFile(marker, os.O_WRONLY|os.O_CREATE|os.O_EXCL, 0o600)
	if err != nil {
		_ = os.RemoveAll(home)
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	if err := secureDesktopControlFile(file); err != nil {
		_ = file.Close()
		_ = os.RemoveAll(home)
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	if _, err := file.Write(data); err != nil {
		_ = file.Close()
		_ = os.RemoveAll(home)
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	if err := file.Close(); err != nil {
		_ = os.RemoveAll(home)
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	_ = os.RemoveAll(aside)
}

func failDesktopControlFixture(t *testing.T, err error) {
	t.Helper()
	if os.Getenv("CI") != "" {
		t.Fatalf("private Windows fixture: %v", err)
	}
	if err == syscall.ERROR_ACCESS_DENIED {
		t.Skipf("private Windows fixture unavailable: %v", err)
	}
	t.Fatal("private Windows fixture", err)
}

// secureDesktopControlPath sets the private control DACL on an existing path
// (directory or file) the fixture just created and therefore owns.
func secureDesktopControlPath(path string) error {
	sid, err := desktopControlCurrentUserSID()
	if err != nil {
		return err
	}
	acl, descriptor, err := desktopControlParseDACL(desktopControlPrivateSDDL(sid, ""))
	if err != nil {
		return err
	}
	defer desktopFreeSecurity.Call(descriptor)
	pathPtr, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	// SE_FILE_OBJECT; DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION.
	status, _, _ := desktopSetNamedSecurity.Call(uintptr(unsafe.Pointer(pathPtr)), 1, 0x80000004, 0, 0, acl, 0)
	runtime.KeepAlive(pathPtr)
	if status != 0 {
		return syscall.Errno(status)
	}
	return nil
}
