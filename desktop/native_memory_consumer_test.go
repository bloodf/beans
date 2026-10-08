package main

import (
	"context"
	"encoding/json"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"

	"github.com/bloodf/beans/desktop/model"
	"github.com/coder/websocket"
	"github.com/egoist/mygo/ui"
)

func TestNativeSetupCallbackRendersOnlyValidatedPreview(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	reply := `{"action":"memory.lance.binding.preview","token":"token","expires_at":9999999999,"runner_id":"r","bot_id":"b","connection_revision":{"counter":1,"device_id":"d"},"profile_id":null,"profile_revision":null,"details":{"directory":"/fixture","create":false,"table":"beans_memory_v1","namespace":"` + strings.Repeat("a", 64) + `"},"unexpected_secret":"SENTINEL"}`
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		conn, err := websocket.Accept(w, req, nil)
		if err != nil {
			return
		}
		defer conn.CloseNow()
		_, data, err := conn.Read(req.Context())
		if err != nil {
			return
		}
		var frame struct {
			ID int `json:"id"`
		}
		_ = json.Unmarshal(data, &frame)
		_ = conn.Write(req.Context(), websocket.MessageText, jsonBytes(map[string]any{"id": frame.ID, "result": json.RawMessage(reply)}))
		<-req.Context().Done()
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	conn, _, err := websocket.Dial(ctx, "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if err != nil {
		t.Fatal(err)
	}
	defer conn.CloseNow()
	oldApp := app
	oldExecutor := executeMain
	queue := make(chan func(), 4)
	executeMain = func(fn func()) { queue <- fn }
	defer func() { waitMainIdle(); executeMain = oldExecutor; app = oldApp }()
	client := newCLIClient()
	client.conn = conn
	client.state = "connected"
	client.generation = 1
	app = &appDelegate{cli: client}
	go client.read(1, conn)
	n := &nativeDesktop{store: model.NewNativeStore(), memory: &nativeMemory{runtime: r}}
	n.store.Connected = true
	n.store.Bots = []model.NativeBot{{ID: "b", RunnerID: "r"}}
	n.authority = client.captureAuthority()
	n.memoryPanel.Connections = json.RawMessage(`{"connections":[{"id":"c","availability":"supported","revision":{"counter":1,"device_id":"d"}}]}`)
	n.memoryPanel.Forms.Bot = &memoryBotForm{ID: "b", Connection: "c", SetupMethod: "memory.lance.binding.preview", Path: "/fixture", Saved: json.RawMessage(`{"connection_id":"c"}`)}
	n.previewMemorySetup()
	select {
	case fn := <-queue:
		fn()
	case <-ctx.Done():
		t.Fatal("preview callback missing")
	}
	b := n.memoryPanel.Forms.Bot
	if b.Approval == 0 || strings.Contains(string(b.Preview), "SENTINEL") {
		t.Fatalf("unsafe consumer preview: %s %s", b.Preview, n.memoryPanel.Forms.Error)
	}
	tt := ui.NewTester(func(c *ui.Context) { n.botMemoryControls(c, b) }, 800, 1600)
	if tt.HasText("SENTINEL") || !tt.HasText("/fixture") {
		t.Fatal("UI bypassed validated preview")
	}
}
func waitMainIdle() {
	for {
		mainPosts.Lock()
		idle := !mainPosts.running
		mainPosts.Unlock()
		if idle {
			return
		}
		time.Sleep(time.Millisecond)
	}
}

func TestNativeOperationsKeepMetadataWithoutUnnegotiatedActions(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	n := &nativeDesktop{memory: &nativeMemory{runtime: r}}
	b := &memoryBotForm{Operations: json.RawMessage(`{"operations":[{"id":"op","state":"queued"}]}`), Health: json.RawMessage(`{"capabilities":{"operation_status":false,"cancel_operation":false}}`)}
	n.memoryPanel.Forms.Bot = b
	tt := ui.NewTester(func(c *ui.Context) { n.botMemoryControls(c, b) }, 800, 1600)
	if !tt.HasText("op · queued") || tt.HasText("Operation status") || tt.HasText("Cancel operation…") {
		t.Fatal(tt.Texts())
	}
	n.memoryOperation("op", "cancel")
	if n.memoryPanel.Forms.Pending || n.memoryPanel.Forms.Error == "" {
		t.Fatal("dispatch bypassed negotiation")
	}
	b.Health = json.RawMessage(`{"capabilities":{"operation_status":true,"cancel_operation":false}}`)
	tt.Frame()
	if !tt.HasText("Operation status") || tt.HasText("Cancel operation…") {
		t.Fatal("capabilities not independent")
	}
}
