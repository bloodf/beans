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
	"github.com/egoist/mygo/ui"
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

func TestNativeFirstFrameSelectedSidebar(t *testing.T) {
	n := &nativeDesktop{store: model.NewNativeStore()}
	session := model.NewSession(func(bool) {}, n.event)
	old := session.Connect()
	session.Disconnect()
	current := session.Connect()
	snapshot := json.RawMessage(`{"has_identity":true,"identity_id":"fixture","chats":[{"id":"selected","title":"Selected fixture","messages":[{"id":"one","author":{"kind":"you"},"body":{"kind":"text","text":"SELECTED TRANSCRIPT"},"state":{"kind":"complete"}}]},{"id":"second","title":"Second fixture","messages":[{"id":"two","author":{"kind":"you"},"body":{"kind":"text","text":"SECOND TRANSCRIPT"},"state":{"kind":"complete"}}]}]}`)
	if err := session.Bootstrap(current, snapshot); err != nil {
		t.Fatal(err)
	}
	first := ui.NewTester(n.view, 1100, 760)
	if !first.HasText("SELECTED TRANSCRIPT") || first.HasText("SECOND TRANSCRIPT") {
		t.Fatal("first transcript disagrees with admitted selection")
	}
	r, ok := first.Find("Selected fixture")
	if !ok {
		t.Fatal("missing first sidebar row")
	}
	image := first.Image()
	selectedColor := image.RGBAAt(int(r.X+5), int(r.Y+r.H/2))
	n.store.Selected = "second"
	second := ui.NewTester(n.view, 1100, 760)
	r2, ok := second.Find("Selected fixture")
	if !ok {
		t.Fatal("missing distinct sidebar row")
	}
	unselectedColor := second.Image().RGBAAt(int(r2.X+5), int(r2.Y+r2.H/2))
	if selectedColor == unselectedColor {
		t.Fatal("first-frame sidebar does not visually mark selected chat")
	}
	if !second.HasText("SECOND TRANSCRIPT") || second.HasText("SELECTED TRANSCRIPT") {
		t.Fatal("second selection transcript mismatch")
	}
	if err := session.Bootstrap(old, snapshot); err != nil {
		t.Fatal(err)
	}
	second.Frame()
	if n.store.Selected != "second" || !second.HasText("SECOND TRANSCRIPT") {
		t.Fatal("stale bootstrap replaced selection/transcript")
	}
}
