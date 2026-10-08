package main

import (
	"context"
	"encoding/json"
	"encoding/json/jsontext"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/bloodf/beans/desktop/model"
	"github.com/coder/websocket"
)

func TestNativeHostReconnectAdmission(t *testing.T) {
	s := model.NewNativeStore()
	session := model.NewSession(func(bool) {}, func(name string, data json.RawMessage) { s.Apply(name, data) })
	oldNative := native
	defer func() { native = oldNative }()
	native = &nativeDesktop{store: s}
	host := &appDelegate{cli: newCLIClient(), session: session}
	connect := func() uint64 { return host.connectionTransition("connected") }
	snapshot := func(account string) json.RawMessage {
		return json.RawMessage(`{"has_identity":true,"identity_id":"` + account + `","chats":[{"id":"c"}]}`)
	}
	generation := connect()
	session.Bootstrap(generation, snapshot("old"))
	s.Draft("c").Text = "private"
	host.connectionTransition("disconnected")
	generation = connect()
	if _, _, err := s.BeginSend("c", "blocked"); err == nil {
		t.Fatal("pending bootstrap enabled stale send")
	}
	session.Bootstrap(generation, json.RawMessage(`{}`))
	if s.Connected {
		t.Fatal("malformed bootstrap admitted account")
	}
	session.Bootstrap(generation, snapshot("old"))
	if !s.Connected || s.Draft("c").Text != "private" {
		t.Fatal("same-account reconnect lost draft")
	}
	host.connectionTransition("disconnected")
	generation = connect()
	session.Bootstrap(generation, snapshot("new"))
	if s.Draft("c").Text != "" || s.ArchivedDrafts["old"]["c"].Text != "private" {
		t.Fatal("draft crossed account or was discarded")
	}
	host.connectionTransition("disconnected")
	generation = connect()
	session.Bootstrap(generation, json.RawMessage(`{"has_identity":false}`))
	if s.Connected || s.HasIdentity {
		t.Fatal("no-identity bootstrap enabled mutations")
	}
}

func TestNativeDelayedDispatchCannotWriteReplacementSocket(t *testing.T) {
	writes := make(chan string, 2)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		conn, err := websocket.Accept(w, r, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		_, data, err := conn.Read(r.Context())
		if err == nil {
			writes <- string(data)
		}
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(ctx, "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	c := newCLIClient()
	c.conn = conn
	c.state = "connected"
	c.generation = 1
	captured := c.captureAuthority()
	resume := make(chan struct{})
	finished := make(chan error, 1)
	go func() {
		<-resume
		_, err := c.requestBound(ctx, "chats.send", jsontext.Value(`{"chat_id":"old","attachments":[{"path":"/old/private"}]}`), &captured)
		finished <- err
	}()
	c.mu.Lock()
	c.generation = 2
	c.authority++
	c.mu.Unlock()
	close(resume)
	if err := <-finished; err != errConnectionClosed {
		t.Fatalf("dispatch accepted changed authority: %v", err)
	}
	// A current probe proves the replacement socket is live and carries only new authority.
	if err := conn.Write(ctx, websocket.MessageText, []byte(`{"current":true}`)); err != nil {
		t.Fatal(err)
	}
	if got := <-writes; got != `{"current":true}` {
		t.Fatalf("old payload reached replacement: %s", got)
	}
}

func TestNativeClosedWindowDraftsBlockQuitAndInstall(t *testing.T) {
	previousExecutor := executeMain
	executeMain = func(fn func()) { fn() }
	defer func() { executeMain = previousExecutor }()
	old := native
	defer func() { native = old }()
	for _, draft := range []*model.NativeDraft{{Text: "private"}, {Attachments: []model.NativeAttachment{{Path: "/private"}}}, {ReplyTo: "m"}, {Sending: true}} {
		native = &nativeDesktop{store: model.NewNativeStore()}
		native.store.Drafts["c"] = draft
		if native.win != nil {
			t.Fatal("fixture must have no native window")
		}
		if err := nativeQuitAdmission(); err == nil {
			t.Fatal("ordinary quit discarded intent")
		}
		if _, err := desktopUpdateIdle(context.Background(), nil); err == nil || !strings.Contains(err.Error(), "native drafts") {
			t.Fatalf("install admission: %v", err)
		}
		if err := (&desktopUpdateLease{}).guardPages(context.Background()); err == nil {
			t.Fatal("lease bypassed closed-window draft")
		}
	}
}
