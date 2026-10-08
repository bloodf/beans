package main

import (
	"context"
	"encoding/json"
	"github.com/bloodf/beans/desktop/model"
	"github.com/coder/websocket"
	"github.com/egoist/mygo/ui"
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
	"time"
)

func TestNativeMemoryNoteMountedConflict(t *testing.T) {
	requests := make(chan json.RawMessage, 4)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
		c, e := websocket.Accept(w, r, nil)
		if e != nil {
			return
		}
		defer c.CloseNow()
		for i := 0; i < 3; i++ {
			_, raw, e := c.Read(r.Context())
			if e != nil {
				return
			}
			requests <- raw
			var q struct {
				ID int `json:"id"`
			}
			json.Unmarshal(raw, &q)
			reply := map[string]any{"id": q.ID}
			switch i {
			case 0:
				reply["result"] = map[string]any{"bot_id": "b", "index": map[string]any{"text": "original", "hash": "h1", "max_lines": 200, "max_bytes": 24000}}
			case 1:
				reply["error"] = map[string]string{"message": "MEMORY.md changed since you opened it"}
			case 2:
				reply["result"] = map[string]string{"hash": "h2"}
			}
			c.Write(r.Context(), websocket.MessageText, jsonBytes(reply))
		}
		<-r.Context().Done()
	}))
	defer server.Close()
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	conn, _, e := websocket.Dial(ctx, "ws"+strings.TrimPrefix(server.URL, "http"), nil)
	if e != nil {
		t.Fatal(e)
	}
	defer conn.CloseNow()
	oldApp, oldExecute := app, executeMain
	queue := make(chan func(), 4)
	executeMain = func(fn func()) { queue <- fn }
	defer func() { waitMainIdle(); app = oldApp; executeMain = oldExecute }()
	client := newCLIClient()
	client.conn = conn
	client.state = "connected"
	client.generation = 1
	app = &appDelegate{cli: client}
	go client.read(1, conn)
	n := &nativeDesktop{store: model.NewNativeStore()}
	n.store.Connected = true
	n.store.AccountID = "a"
	n.store.Bots = []model.NativeBot{{ID: "b", Name: "Fixture"}}
	n.authority = client.captureAuthority()
	n.memoryPanel.Open = true
	n.memoryPanel.Connections = json.RawMessage(`{"connections":[],"embeddings":[]}`)
	tt := ui.NewTester(n.memoryView, 1000, 1800)
	if e = tt.Click("MEMORY.md for Fixture"); e != nil {
		t.Fatal(e)
	}
	step := func(method string) map[string]any {
		t.Helper()
		var q map[string]any
		select {
		case raw := <-requests:
			json.Unmarshal(raw, &q)
		case <-ctx.Done():
			t.Fatal("missing request")
		}
		if q["method"] != method {
			t.Fatal(q["method"])
		}
		select {
		case fn := <-queue:
			fn()
		case <-ctx.Done():
			t.Fatal("missing completion")
		}
		tt.Frame()
		return q
	}
	step("bots.memory")
	d := n.memoryPanel.Forms.Note
	if d.Text != "original" || !n.hasMemoryIntent() {
		t.Fatal("load/ownership missing")
	}
	d.Text = "my edit"
	tt.Frame()
	if e = tt.Click("Save memory note"); e != nil {
		t.Fatal(e)
	}
	_ = tt.Click("Cancel note")
	if n.memoryPanel.Forms.Note != d {
		t.Fatal("pending cancelled")
	}
	q := step("bots.memory.write")
	if q["params"].(map[string]any)["expected_hash"] != "h1" || !d.Conflict || d.Text != "my edit" {
		t.Fatal("hash conflict discarded draft")
	}
	if e = tt.Click("Overwrite with mine…"); e != nil {
		t.Fatal(e)
	}
	tt.Frame()
	if e = tt.Click("Confirm overwrite memory note"); e != nil {
		t.Fatal(e)
	}
	q = step("bots.memory.write")
	if _, ok := q["params"].(map[string]any)["expected_hash"]; ok {
		t.Fatal("explicit overwrite kept hash")
	}
	if n.memoryPanel.Forms.Note != nil {
		t.Fatal("save not closed")
	}
	n.memoryPanel.Forms.Note = &memoryNoteForm{BotID: "b", Text: strings.Repeat("x", nativeMemoryNoteMaxBytes+1), Loaded: true, Hash: "h2"}
	n.saveMemoryNote(false)
	if n.memoryPanel.Forms.Pending || n.memoryPanel.Forms.Error == "" {
		t.Fatal("oversized dispatched")
	}
	n.suspendMemoryForms()
	if !n.hasMemoryIntent() {
		t.Fatal("recovery lost quit veto")
	}
}
