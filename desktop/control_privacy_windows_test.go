//go:build windows

package main

import (
	"bytes"
	"os"
	"path/filepath"
	"runtime"
	"syscall"
	"testing"
	"unsafe"
)

func fixtureControlDACL(t *testing.T, file *os.File, extra string) {
	t.Helper()
	token, err := syscall.OpenCurrentProcessToken()
	if err != nil {
		t.Fatal(err)
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil {
		t.Fatal(err)
	}
	sid, err := user.User.Sid.String()
	if err != nil {
		t.Fatal(err)
	}
	text, err := syscall.UTF16PtrFromString("D:P(A;;FA;;;" + sid + ")" + extra)
	if err != nil {
		t.Fatal(err)
	}
	var descriptor uintptr
	ok, _, callErr := desktopConvertSecurity.Call(uintptr(unsafe.Pointer(text)), 1, uintptr(unsafe.Pointer(&descriptor)), 0)
	runtime.KeepAlive(text)
	if ok == 0 {
		t.Fatal(callErr)
	}
	defer desktopFreeSecurity.Call(descriptor)
	var present, defaulted uint32
	var acl uintptr
	ok, _, callErr = desktopReadDACL.Call(descriptor, uintptr(unsafe.Pointer(&present)), uintptr(unsafe.Pointer(&acl)), uintptr(unsafe.Pointer(&defaulted)))
	if ok == 0 || present == 0 || acl == 0 {
		t.Fatal("fixture DACL", callErr)
	}
	handle, _, callErr := desktopReopenControl.Call(file.Fd(), 0x40000, 7, 0x02000000)
	if handle == ^uintptr(0) {
		t.Fatal(callErr)
	}
	defer syscall.CloseHandle(syscall.Handle(handle))
	status, _, _ := desktopSetSecurity.Call(handle, 1, 0x80000004, 0, 0, acl, 0)
	if status != 0 {
		t.Fatal(syscall.Errno(status))
	}
}

func fixtureControlSecurity(t *testing.T, file *os.File) []byte {
	t.Helper()
	var descriptor uintptr
	status, _, _ := desktopGetSecurity.Call(file.Fd(), 1, 5, 0, 0, 0, 0, uintptr(unsafe.Pointer(&descriptor)))
	if status != 0 {
		t.Fatal(syscall.Errno(status))
	}
	defer desktopFreeSecurity.Call(descriptor)
	length, _, _ := desktopDescriptorLength.Call(descriptor)
	return bytes.Clone(unsafe.Slice((*byte)(unsafe.Pointer(descriptor)), int(length)))
}

func TestWindowsExistingControlRefusesInsecureObjectsUntouched(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_UPDATE_TOKEN_FILE")
	home := t.TempDir()
	t.Setenv("BEANS_HOME", home)
	if err := os.WriteFile(filepath.Join(home, "format.json"), []byte(`{"format":"beans-v2"}`), 0o600); err != nil {
		t.Fatal(err)
	}
	prepareDesktopTestControlHome(t, home)
	path, err := ensureDesktopUpdateToken()
	if err != nil {
		t.Fatal(err)
	}
	for _, test := range []struct{ name, path, grant string }{
		{"token readable by Everyone", path, "(A;;FR;;;WD)"},
		{"token readable by Administrators", path, "(A;;FR;;;BA)"},
		{"directory writable by Everyone", home, "(A;;GW;;;WD)"},
	} {
		t.Run(test.name, func(t *testing.T) {
			file, err := os.Open(test.path)
			if err != nil {
				t.Fatal(err)
			}
			defer file.Close()
			fixtureControlDACL(t, file, test.grant)
			before := fixtureControlSecurity(t, file)
			info, err := file.Stat()
			if err != nil {
				t.Fatal(err)
			}
			secret, err := os.ReadFile(path)
			if err != nil {
				t.Fatal(err)
			}
			if _, err := ensureDesktopUpdateToken(); err == nil {
				t.Fatal("insecure existing object admitted for provisioning")
			}
			if _, err := desktopUpdateToken(); err == nil {
				t.Fatal("insecure existing object admitted for installation")
			}
			after, err := os.ReadFile(path)
			if err != nil || !bytes.Equal(secret, after) {
				t.Fatal("token bytes changed", err)
			}
			current, err := os.Stat(test.path)
			if err != nil || !os.SameFile(info, current) || !bytes.Equal(before, fixtureControlSecurity(t, file)) {
				t.Fatal("existing inode/owner/DACL changed", err)
			}
			// Reset only this disposable fixture for the next independent case.
			fixtureControlDACL(t, file, "")
		})
	}
}
