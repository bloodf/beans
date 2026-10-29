//go:build windows

package main

import (
	"os"
	"path/filepath"
	"syscall"
	"testing"
	"unsafe"
)

// prepareWindowsDesktopTestControlHome applies the private control DACL to a
// disposable home and its format marker. Production never repairs existing
// objects; fixtures recreate the directory and marker with the private SDDL at
// create time so GitHub-hosted runners (which refuse WRITE_DAC on TempDir
// inheritance) still get a private home.
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
	if err := createDesktopControlDirectory(home); err != nil {
		_ = os.Rename(aside, home)
		failDesktopControlFixture(t, err)
	}
	if err := createDesktopControlFile(marker, data); err != nil {
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

func desktopControlSecurityAttributes(extra string) (*syscall.SecurityAttributes, uintptr, error) {
	sid, err := desktopControlCurrentUserSID()
	if err != nil {
		return nil, 0, err
	}
	_, descriptor, err := desktopControlParseDACL(desktopControlPrivateSDDL(sid, extra))
	if err != nil {
		return nil, 0, err
	}
	sa := &syscall.SecurityAttributes{
		Length:             uint32(unsafe.Sizeof(syscall.SecurityAttributes{})),
		SecurityDescriptor: descriptor,
		InheritHandle:      0,
	}
	return sa, descriptor, nil
}

func createDesktopControlDirectory(path string) error {
	sa, descriptor, err := desktopControlSecurityAttributes("")
	if err != nil {
		return err
	}
	defer desktopFreeSecurity.Call(descriptor)
	pathPtr, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	return syscall.CreateDirectory(pathPtr, sa)
}

func createDesktopControlFile(path string, data []byte) error {
	sa, descriptor, err := desktopControlSecurityAttributes("")
	if err != nil {
		return err
	}
	defer desktopFreeSecurity.Call(descriptor)
	pathPtr, err := syscall.UTF16PtrFromString(path)
	if err != nil {
		return err
	}
	handle, err := syscall.CreateFile(pathPtr, syscall.GENERIC_WRITE, 0, sa, syscall.CREATE_NEW, syscall.FILE_ATTRIBUTE_NORMAL, 0)
	if err != nil {
		return err
	}
	defer syscall.CloseHandle(handle)
	var written uint32
	return syscall.WriteFile(handle, data, &written, nil)
}
