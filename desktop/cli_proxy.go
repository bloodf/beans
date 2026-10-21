package main

import (
    "context"
    "io"
    "net"
    "net/http"
    "os"
    "fmt"
    "bufio"
    "crypto/subtle"
    "time"

    "github.com/coder/websocket"
)

// Vite delegates private file admission to the existing native reader on every platform.
// This development-only broker independently checks browser authority before dialing.

func runDevelopmentCLIProxy() error {
    if !isDevelopment() || os.Getenv("BEANS_CLI_DEV_PROXY") != "1" { return fmt.Errorf("Native development broker requires an explicit development build") }
    if _, set := os.LookupEnv("BEANS_HOME"); !set {
        home, err := os.UserHomeDir()
        if err != nil { return err }
        if err := os.Setenv("BEANS_HOME", home+string(os.PathSeparator)+".beans-dev-v2"); err != nil { return err }
    }
    input := bufio.NewReaderSize(os.Stdin, 128)
    secret, err := input.ReadString('\n')
    if err != nil || len(secret) != 65 { return fmt.Errorf("Missing private development broker credential") }
    secret = secret[:64]
    listener, err := net.Listen("tcp", "127.0.0.1:0")
    if err != nil { return err }
    server := &http.Server{ReadHeaderTimeout: 5 * time.Second, Handler: http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
        values := r.Header.Values("Beans-Dev-Proxy")
        if len(values) != 1 || subtle.ConstantTimeCompare([]byte(values[0]), []byte(secret)) != 1 {
            http.Error(w, "Forbidden", http.StatusForbidden)
            return
        }
        r.Header.Del("Beans-Dev-Proxy")
        developmentCLIUpgrade(w, r)
    })}
    go func() { _, _ = io.Copy(io.Discard, input); _ = server.Close() }()
    fmt.Printf("beans-cli-proxy-ready %d\n", listener.Addr().(*net.TCPAddr).Port)
    err = server.Serve(listener)
    if err == http.ErrServerClosed { return nil }
    return err
}
func developmentCLIUpgrade(w http.ResponseWriter, r *http.Request) {
    peer, _, err := net.SplitHostPort(r.RemoteAddr)
    ip := net.ParseIP(peer)
    origin := r.Header.Values("Origin")
    if err != nil || ip == nil || !ip.IsLoopback() || r.URL.Path != "/__beans_cli_ws" || r.URL.RawQuery != "" ||
        (r.Host != "localhost:5178" && r.Host != "127.0.0.1:5178") || len(origin) != 1 || origin[0] != "http://"+r.Host || len(r.Header.Values("Authorization")) != 0 {
        http.Error(w, "Forbidden", http.StatusForbidden)
        return
    }
    headers, err := cliAuthorization()
    if err != nil { http.Error(w, "CLI unavailable", http.StatusServiceUnavailable); return }
    headers.Set("Origin", origin[0])
    ctx, cancel := context.WithCancel(r.Context())
    defer cancel()
    upstream, _, err := websocket.Dial(ctx, "ws://127.0.0.1:4875/ws", &websocket.DialOptions{HTTPHeader: headers})
    if err != nil { http.Error(w, "CLI unavailable", http.StatusServiceUnavailable); return }
    defer upstream.CloseNow()
    client, err := websocket.Accept(w, r, &websocket.AcceptOptions{OriginPatterns: []string{r.Host}})
    if err != nil { return }
    defer client.CloseNow()
    client.SetReadLimit(256 * 1024 * 1024)
    upstream.SetReadLimit(256 * 1024 * 1024)
    copyFrames := func(to, from *websocket.Conn) {
        for {
            kind, reader, err := from.Reader(ctx)
            if err != nil { cancel(); return }
            writer, err := to.Writer(ctx, kind)
            if err != nil { cancel(); return }
            _, err = io.Copy(writer, reader)
            closeErr := writer.Close()
            if err != nil || closeErr != nil { cancel(); return }
        }
    }
    go copyFrames(upstream, client)
    copyFrames(client, upstream)
}

