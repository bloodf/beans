//go:build windows

package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"syscall"
	"testing"
)

// prepareWindowsDesktopTestControlHome applies the private control DACL to a
// disposable home and its format marker. Production never repairs existing
// objects; fixtures recreate a creator-owned directory and exclusive marker,
// then apply the shared private SDDL via icacls (GitHub-hosted runners refuse
// SetNamedSecurityInfo / ReOpenFile WRITE_DAC even on freshly mkdir'd TempDir
// children).
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

// secureDesktopControlPath sets a protected DACL granting only the current
// user full control, matching desktopControlPrivateSDDL.
func secureDesktopControlPath(path string) error {
	sid, err := desktopControlCurrentUserSID()
	if err != nil {
		return err
	}
	// /inheritance:r drops inherited TempDir ACEs; /grant:r replaces grants for
	// this SID with Full Control. SYSTEM is added so LocalSystem remains a
	// trusted owner/grantee under desktopControlACLPrivate.
	grant := "*" + sid + ":(F)"
	system := "*S-1-5-18:(F)"
	cmd := exec.Command("icacls", path, "/inheritance:r", "/grant:r", grant, "/grant:r", system)
	out, err := cmd.CombinedOutput()
	if err != nil {
		return fmt.Errorf("icacls: %w: %s", err, out)
	}
	return nil
}
