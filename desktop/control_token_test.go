package main

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"
	"testing"
)

func unsetDesktopTestEnv(t *testing.T, key string) {
	t.Helper()
	t.Setenv(key, "")
	if err := os.Unsetenv(key); err != nil { t.Fatal(err) }
}

// Only fresh disposable Windows fixtures receive private ACLs. Production
// admission never repairs an existing home or record.
func prepareDesktopTestControlHome(t *testing.T, home string) {
	t.Helper()
	if runtime.GOOS != "windows" { return }
	for _, path := range []string{home, filepath.Join(home, "format.json")} {
		file, err := os.Open(path)
		if err != nil { t.Fatal(err) }
		err = secureDesktopControlFile(file)
		closeErr := file.Close()
		if err != nil || closeErr != nil { t.Fatal("private Windows fixture", err, closeErr) }
	}
}

func TestDesktopTokenDoesNotInitializeHome(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_UPDATE_TOKEN_FILE")
	home := filepath.Join(t.TempDir(), "absent")
	t.Setenv("BEANS_HOME", home)
	_ = newLauncher().environment()
	if _, err := ensureDesktopUpdateToken(); err == nil {
		t.Fatal("an absent home must be established by the core, not update control")
	}
	if _, err := os.Lstat(home); !os.IsNotExist(err) {
		t.Fatalf("update control created or changed the absent home: %v", err)
	}
}

func TestDesktopTokenRejectsUnmarkedOrInvalidHomeUntouched(t *testing.T) {
	for _, marker := range []string{"", `{"format":"beans-v1"}`, `{"format":`, `{"format":"beans-v1","format":"beans-v2"}`} {
		t.Run(marker, func(t *testing.T) {
			unsetDesktopTestEnv(t, "BEANS_UPDATE_TOKEN_FILE")
			home := t.TempDir()
			t.Setenv("BEANS_HOME", home)
			retained := filepath.Join(home, "identity.json")
			if err := os.WriteFile(retained, []byte("retained account bytes"), 0o640); err != nil { t.Fatal(err) }
			before, err := os.Stat(retained)
			if err != nil { t.Fatal(err) }
			if marker != "" {
				if err := os.WriteFile(filepath.Join(home, "format.json"), []byte(marker), 0o600); err != nil { t.Fatal(err) }
			}
			if _, err := ensureDesktopUpdateToken(); err == nil { t.Fatal("incompatible home was admitted") }
			if _, err := os.Lstat(filepath.Join(home, "update-token")); !os.IsNotExist(err) { t.Fatalf("token appeared: %v", err) }
			data, err := os.ReadFile(retained)
			if err != nil || string(data) != "retained account bytes" { t.Fatalf("retained data changed: %q, %v", data, err) }
			after, err := os.Stat(retained)
			if err != nil || after.Mode() != before.Mode() { t.Fatalf("retained mode changed: %v", err) }
			if marker != "" {
				data, err = os.ReadFile(filepath.Join(home, "format.json"))
				if err != nil || string(data) != marker { t.Fatalf("marker changed: %q, %v", data, err) }
			}
		})
	}
}

func TestDesktopTokenUsesEstablishedHomeAndPreservesExistingToken(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_UPDATE_TOKEN_FILE")
	home := t.TempDir()
	t.Setenv("BEANS_HOME", home)
	if err := os.Chmod(home, 0o700); err != nil { t.Fatal(err) }
	// Fixture represents the core's successful fresh admission, not UI initialization.
	if err := os.WriteFile(filepath.Join(home, "format.json"), []byte(`{"format":"beans-v2"}`), 0o600); err != nil { t.Fatal(err) }
	prepareDesktopTestControlHome(t, home)
	path, err := ensureDesktopUpdateToken()
	if err != nil { t.Fatal(err) }
	data, err := os.ReadFile(path)
	if err != nil || len(strings.TrimSpace(string(data))) < 32 { t.Fatalf("invalid private token: %v", err) }
	before, err := os.Stat(path)
	if err != nil { t.Fatal(err) }
	if runtime.GOOS != "windows" && before.Mode().Perm() != 0o600 { t.Fatalf("token permissions: %v", before.Mode()) }
	if _, err = ensureDesktopUpdateToken(); err != nil { t.Fatal(err) }
	afterData, err := os.ReadFile(path)
	if err != nil || string(afterData) != string(data) { t.Fatalf("existing token bytes changed: %v", err) }
	after, err := os.Stat(path)
	if err != nil || !os.SameFile(before, after) || after.Mode() != before.Mode() { t.Fatalf("existing token replaced or chmodded: %v", err) }
}

func TestDesktopTokenPreservesExplicitOperatorConfiguration(t *testing.T) {
	for _, setting := range []string{"", filepath.Join(t.TempDir(), "operator-token")} {
		t.Run(setting, func(t *testing.T) {
			home := filepath.Join(t.TempDir(), "absent")
			t.Setenv("BEANS_HOME", home)
			t.Setenv("BEANS_UPDATE_TOKEN_FILE", setting)
			if setting != "" {
				if err := os.WriteFile(setting, []byte("operator bytes"), 0o644); err != nil { t.Fatal(err) }
			}
			path, err := ensureDesktopUpdateToken()
			if err != nil || path != setting { t.Fatalf("operator setting changed: %q, %v", path, err) }
			if _, err := os.Lstat(home); !os.IsNotExist(err) { t.Fatalf("operator setting caused home writes: %v", err) }
			if setting != "" {
				data, err := os.ReadFile(setting)
				if err != nil || string(data) != "operator bytes" { t.Fatalf("operator bytes changed: %v", err) }
				info, err := os.Stat(setting)
				if err != nil || (runtime.GOOS != "windows" && info.Mode().Perm() != 0o644) { t.Fatalf("operator mode changed: %v", err) }
			}
		})
	}
}

func TestDesktopTokenFollowsOwnedReadyAndExistingMarker(t *testing.T) {
	for _, test := range []struct { name string; marked, current, matching, wantToken bool }{
		{"ready after core admission", true, true, true, true},
		{"ready cannot label unmarked home", false, true, true, false},
		{"stale child", true, false, true, false},
		{"wrong readiness port", true, true, false, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			unsetDesktopTestEnv(t, "BEANS_UPDATE_TOKEN_FILE")
			home := t.TempDir()
			t.Setenv("BEANS_HOME", home)
			if err := os.Chmod(home, 0o700); err != nil { t.Fatal(err) }
			if test.marked {
				if err := os.WriteFile(filepath.Join(home, "format.json"), []byte(`{"format":"beans-v2"}`), 0o600); err != nil { t.Fatal(err) }
				prepareDesktopTestControlHome(t, home)
			}
			path := filepath.Join(home, "update-token")
			command := &exec.Cmd{Env: []string{"BEANS_HOME=" + home, "BEANS_UPDATE_TOKEN_FILE=" + path}}
			launcher := newLauncher()
			launcher.process = command
			launcher.generation = 1
			generation := launcher.generation
			if !test.current { generation = 0 }
			recordPort := defaultCLIPort()
			if !test.matching { recordPort++ }
			// Feed the real startup reader a disposable channel; never spawn a CLI/service.
			launcher.readOutput(generation, defaultCLIPort(), command,
				strings.NewReader(fmt.Sprintf("{\"event\":\"ready\",\"port\":%d}\n", recordPort)), nil, "")
			_, err := os.Lstat(path)
			if test.wantToken && err != nil { t.Fatalf("ready marked child has no token: %v", err) }
			if !test.wantToken && !os.IsNotExist(err) { t.Fatalf("non-admitted startup wrote token: %v", err) }
			if !test.marked {
				if _, err := os.Lstat(filepath.Join(home, "format.json")); !os.IsNotExist(err) { t.Fatalf("UI created a marker: %v", err) }
			}
		})
	}
}
