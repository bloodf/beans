package main

import (
    "encoding/base64"
    "encoding/json/v2"
    "errors"
    "net/http"
    "os"
    "path/filepath"
    "runtime"
    "strings"
)

// cliAuthorization reads only the selected CLI home's transport token. Update
// control credentials remain independent and never select this path.
func cliAuthorization() (http.Header, error) {
    home := cliHome()
    before, err := os.Lstat(home)
    if err != nil || !before.IsDir() || before.Mode()&os.ModeSymlink != 0 { return nil, errors.New("Cannot admit the local CLI home") }
    absolute, err := filepath.Abs(home)
    if err != nil { return nil, err }
    var aliasRoot *os.Root
    var aliasInfo os.FileInfo
    if runtime.GOOS == "darwin" && (absolute == "/var" || strings.HasPrefix(absolute, "/var/")) {
        aliasRoot, err = os.OpenRoot("/")
        if err != nil { return nil, err }
        defer aliasRoot.Close()
        aliasInfo, err = aliasRoot.Lstat("var")
        target, targetErr := aliasRoot.Readlink("var")
        if err != nil || targetErr != nil || target != "private/var" || !desktopServeRootOwner(aliasInfo) { return nil, errors.New("Untrusted macOS system alias") }
        absolute = "/private" + absolute
    }
    volume := filepath.VolumeName(absolute) + string(os.PathSeparator)
    root, err := os.OpenRoot(volume)
    if err != nil { return nil, err }
    roots := []*os.Root{root}
    type link struct { parent *os.Root; name string; info os.FileInfo }
    var links []link
    defer func() { for _, opened := range roots { opened.Close() } }()
    for _, name := range strings.Split(strings.TrimPrefix(absolute, volume), string(os.PathSeparator)) {
        if name == "" || name == "." { continue }
        info, err := root.Lstat(name)
        if err != nil || !info.IsDir() || info.Mode()&os.ModeSymlink != 0 { return nil, errors.New("CLI home ancestry must not contain links") }
        child, err := root.OpenRoot(name)
        if err != nil { return nil, err }
        probe, err := child.Open(".")
        if err != nil { child.Close(); return nil, err }
        admitted, statErr := probe.Stat()
        probe.Close()
        if statErr != nil || !os.SameFile(info, admitted) { child.Close(); return nil, errors.New("CLI home ancestry changed") }
        if aliasRoot != nil && (name == "private" || name == "var") && (!desktopServeRootOwner(admitted) || admitted.Mode().Perm()&0o022 != 0) { child.Close(); return nil, errors.New("Untrusted platform prefix") }
        links = append(links, link{root, name, info})
        roots = append(roots, child)
        root = child
    }
    verify := func() error {
        if aliasRoot != nil {
            current, err := aliasRoot.Lstat("var")
            target, targetErr := aliasRoot.Readlink("var")
            if err != nil || targetErr != nil || target != "private/var" || !os.SameFile(current, aliasInfo) || !desktopServeRootOwner(current) { return errors.New("System alias changed") }
        }
        for _, link := range links {
            current, err := link.parent.Lstat(link.name)
            if err != nil || current.Mode()&os.ModeSymlink != 0 || !os.SameFile(current, link.info) { return errors.New("CLI home ancestry changed") }
        }
        return nil
    }
    directory, err := root.Open(".")
    if err != nil { return nil, err }
    defer directory.Close()
    opened, err := directory.Stat()
    if err != nil || !os.SameFile(before, opened) || validateDesktopControlObject(directory, false) != nil { return nil, errors.New("The local CLI home changed or is unsafe") }
    marker, err := readDesktopControlFile(root, "format.json", false)
    var format struct { Format string `json:"format"` }
    if err != nil || json.Unmarshal(marker, &format) != nil || format.Format != "beans-v2" { return nil, errors.New("The local CLI needs an established Beans v2 home") }
    if err := verify(); err != nil { return nil, err }
    data, err := readDesktopControlFile(root, "serve-token", true)
    if err != nil { return nil, err }
    token := strings.TrimSuffix(string(data), "\n")
    raw, err := base64.RawURLEncoding.DecodeString(token)
    if err != nil || len(raw) != 32 || base64.RawURLEncoding.EncodeToString(raw) != token { return nil, errors.New("Invalid local CLI transport token") }
    if err := verify(); err != nil { return nil, err }
    current, err := os.Lstat(home)
    if err != nil || !os.SameFile(opened, current) { return nil, errors.New("The local CLI home changed") }
    return http.Header{"Authorization": {"Bearer " + token}}, nil
}

func cliHome() string {
    if home, set := os.LookupEnv("BEANS_HOME"); set { return home }
    return defaultCLIHome()
}
