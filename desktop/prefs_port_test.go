package main

import (
	"os"
	"path/filepath"
	"strconv"
	"testing"
)

func TestDesktopPortRejectsExplicitIncompatibleConfiguration(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	expected := defaultCLIPort()
	for _, saved := range []int{0, -1, 4864, 4874, 4875, 65536} {
		if saved == expected { continue }
		if port := cliPortLocked(Preferences{CLIPort: saved}); port != 0 {
			t.Fatalf("saved port %d admitted as %d; incompatible settings must remain unusable", saved, port)
		}
	}
	for _, raw := range []string{"", "0", "4864", "4874", "4875", "garbage", " " + strconv.Itoa(expected), strconv.Itoa(expected) + ".0"} {
		if raw == strconv.Itoa(expected) { continue }
		t.Run(raw, func(t *testing.T) {
			t.Setenv("BEANS_PORT", raw)
			if port := cliPortLocked(Preferences{CLIPort: expected}); port != 0 {
				t.Fatalf("explicit BEANS_PORT=%q admitted as %d or fell back", raw, port)
			}
		})
	}
	t.Setenv("BEANS_PORT", strconv.Itoa(expected))
	if port := cliPortLocked(Preferences{CLIPort: expected}); port != expected { t.Fatalf("matching build port rejected: %d", port) }
	t.Setenv("BEANS_PORT", "+" + strconv.Itoa(expected))
	if port := cliPortLocked(Preferences{CLIPort: expected}); port != expected { t.Fatalf("matching integer environment rejected: %d", port) }
	if port := cliPortLocked(Preferences{CLIPort: 4864}); port != 0 { t.Fatalf("valid environment masked incompatible saved port: %d", port) }
}

func TestDesktopRejectedPortPatchPreservesPreferences(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	path := filepath.Join(t.TempDir(), "preferences-v2.json")
	before := []byte(`{"cliPort":4875,"selection":"retained"}`)
	if err := os.WriteFile(path, before, 0o640); err != nil { t.Fatal(err) }
	store := &prefsStore{path: path, value: Preferences{CLIPort: defaultCLIPort(), Selection: "retained"}}
	badPort := 4864
	changed := "must not persist"
	if _, err := store.update(PreferencesPatch{CLIPort: &badPort, Selection: &changed}); err == nil { t.Fatal("incompatible port patch succeeded") }
	data, err := os.ReadFile(path)
	if err != nil || string(data) != string(before) { t.Fatalf("rejected patch changed stored bytes: %v", err) }
	if store.value.Selection != "retained" || store.value.CLIPort != defaultCLIPort() { t.Fatal("rejected patch changed in-memory preferences") }
}

func TestDesktopInvalidPortHasNoUsableConnectionEndpoint(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	t.Setenv("BEANS_PORT", "4864")
	store := &prefsStore{value: Preferences{CLIPort: defaultCLIPort()}}
	port, err := store.cliPort()
	if err == nil || port != 0 {
		t.Fatalf("incompatible environment yielded an endpoint: %d, %v", port, err)
	}
	// Exercise the admission boundary without probing any installed CLI.
	t.Setenv("BEANS_PORT", strconv.Itoa(defaultCLIPort()))
	store.value.CLIPort = 4864
	if port, err := store.cliPort(); err == nil || port != 0 {
		t.Fatalf("incompatible saved preferences yielded an endpoint: %d, %v", port, err)
	}
}
