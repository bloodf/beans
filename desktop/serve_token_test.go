package main

import (
    "encoding/base64"
    "os"
    "path/filepath"
    "testing"
)

func TestServeCredentialIsIndependentAndRejectsCorruptToken(t *testing.T) {
    home := t.TempDir()
    t.Setenv("BEANS_HOME", home)
    t.Setenv("BEANS_UPDATE_TOKEN_FILE", filepath.Join(home, "unrelated-update-token"))
    if err := os.Chmod(home, 0o700); err != nil { t.Fatal(err) }
    if err := os.WriteFile(filepath.Join(home, "format.json"), []byte(`{"format":"beans-v2"}`), 0o600); err != nil { t.Fatal(err) }
    token := base64.RawURLEncoding.EncodeToString(make([]byte, 32))
    path := filepath.Join(home, "serve-token")
    if err := os.WriteFile(path, []byte(token), 0o600); err != nil { t.Fatal(err) }
    for _, name := range []string{home, filepath.Join(home, "format.json"), path} {
        file, err := os.OpenFile(name, os.O_RDONLY, 0)
        if err != nil { t.Fatal(err) }
        err = secureDesktopControlFile(file)
        file.Close()
        if err != nil { t.Fatal(err) }
    }
    headers, err := cliAuthorization()
    if err != nil { t.Fatal(err) }
    if headers.Get("Authorization") != "Bearer "+token { t.Fatal("transport credential not selected") }
    if err := os.WriteFile(path, []byte("invalid"), 0o600); err != nil { t.Fatal(err) }
    if _, err := cliAuthorization(); err == nil { t.Fatal("malformed token accepted") }
    bytes, err := os.ReadFile(path)
    if err != nil || string(bytes) != "invalid" { t.Fatal("reader repaired rejected token") }
}
