//go:build windows

package main

import (
	"fmt"
	"os"
	"os/exec"
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
// take ownership (GHA admin TempDir children are often owned by Administrators),
// then replace the DACL with desktopControlPrivateSDDL via SetNamedSecurityInfo.
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
	if err := secureDesktopControlPath(marker); err != nil {
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

// secureDesktopControlPath takes ownership of path then replaces its DACL with
// the protected private-control SDDL (user full control only). takeown is
// required on GHA so WRITE_DAC succeeds; SetNamedSecurityInfo then drops any
// Administrators ACE takeown may have added.
func secureDesktopControlPath(path string) error {
	sid, err := desktopControlCurrentUserSID()
	if err != nil {
		return err
	}
	if out, err := exec.Command("takeown", "/F", path).CombinedOutput(); err != nil {
		return fmt.Errorf("takeown: %w: %s", err, out)
	}
	if out, err := exec.Command("icacls", path, "/setowner", "*"+sid).CombinedOutput(); err != nil {
		return fmt.Errorf("icacls setowner: %w: %s", err, out)
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
