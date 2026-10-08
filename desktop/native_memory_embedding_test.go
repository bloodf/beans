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

func TestNativeEmbeddingEditorConsumer(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	n := &nativeDesktop{store: model.NewNativeStore(), memory: &nativeMemory{runtime: r}}
	requests := make(chan json.RawMessage, 1)
	server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, req *http.Request) {
		conn, e := websocket.Accept(w, req, nil)
		if e != nil {
			return
		}
		defer conn.CloseNow()
		_, data, e := conn.Read(req.Context())
		if e != nil {
			return
		}
		requests <- data
		var frame struct {
			ID int `json:"id"`
		}
		json.Unmarshal(data, &frame)
		conn.Write(req.Context(), websocket.MessageText, jsonBytes(map[string]any{"id": frame.ID, "error": map[string]string{"message": "fixture rejection"}}))
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
	oldApp, oldExecutor := app, executeMain
	queue := make(chan func(), 4)
	executeMain = func(fn func()) { queue <- fn }
	defer func() { waitMainIdle(); app = oldApp; executeMain = oldExecutor }()
	client := newCLIClient()
	client.conn = conn
	client.state = "connected"
	client.generation = 1
	app = &appDelegate{cli: client}
	n.authority = client.captureAuthority()
	go client.read(1, conn)
	n.store.AccountID = "a"
	n.store.Connected = true
	n.memoryPanel.Open = true
	n.memoryPanel.Connections = json.RawMessage(`{"connections":[],"embeddings":[{"id":"p","model":"fixture","model_revision":"rev","dimensions":3,"has_secret":true}]}`)
	tt := ui.NewTester(n.memoryView, 900, 2200)
	if err := tt.Click("Edit embedding profile"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	d := n.memoryPanel.Forms.Embedding
	if d == nil || d.ID != "p" || d.Secret != "" || d.SecretMode != "keep" {
		t.Fatal("masked edit not opened")
	}
	if !n.hasMemoryIntent() {
		t.Fatal("editor bypassed quit guard")
	}
	d.Values["endpoint"] = "https://example.invalid/embeddings"
	d.SecretMode = "replace"
	d.Secret = " exact secret "
	params, err := n.embeddingParams(d)
	if err != nil {
		t.Fatal(err)
	}
	var body struct {
		Secret  struct{ Action, Value string }
		Profile struct {
			Model      string
			Dimensions int
			Endpoint   string
		}
	}
	if err = json.Unmarshal(params, &body); err != nil {
		t.Fatal(err)
	}
	if body.Secret.Value != d.Secret || body.Profile.Model != "fixture" || body.Profile.Dimensions != 3 {
		t.Fatal("profile or exact secret changed")
	}
	if err := tt.Click("Save profile"); err != nil {
		t.Fatal(err)
	}
	select {
	case raw := <-requests:
		var sent struct {
			Method string
			Params json.RawMessage
		}
		json.Unmarshal(raw, &sent)
		if sent.Method != "memory.embeddings.set" || string(sent.Params) != string(params) {
			t.Fatal("wrong profile dispatch")
		}
	case <-ctx.Done():
		t.Fatal("no request")
	}
	tt.Frame()
	_ = tt.Click("Cancel")
	if n.memoryPanel.Forms.Embedding != d {
		t.Fatal("pending cancel lost draft")
	}
	select {
	case fn := <-queue:
		fn()
	case <-ctx.Done():
		t.Fatal("no ordered failure")
	}
	tt.Frame()
	if !tt.HasText("Embedding profile could not be saved. Review the complete vector space and retry.") || d.Secret != " exact secret " {
		t.Fatal("error discarded draft")
	}
	n.suspendMemoryForms()
	if n.memoryRecovery["a"].Embedding != d || !n.hasMemoryIntent() {
		t.Fatal("reconnect discarded intent")
	}
	n.recoverMemoryForms()
	if n.memoryPanel.Forms.Embedding != d {
		t.Fatal("same account recovery failed")
	}
	d.SecretMode = "keep"
	params, err = n.embeddingParams(d)
	if err != nil {
		t.Fatal(err)
	}
	var obj map[string]any
	json.Unmarshal(params, &obj)
	if _, ok := obj["secret"].(map[string]any)["value"]; ok {
		t.Fatal("keep resends secret")
	}
	d.SecretMode = "clear"
	params, err = n.embeddingParams(d)
	if err != nil {
		t.Fatal(err)
	}
	json.Unmarshal(params, &obj)
	if obj["secret"].(map[string]any)["action"] != "clear" {
		t.Fatal("clear not explicit")
	}
	d.Mode = "local_cpu"
	d.Values["model_sha256"] = strings.Repeat("a", 64)
	d.Values["tokenizer_sha256"] = strings.Repeat("b", 64)
	d.Values["input_ids"] = "ids"
	d.Values["attention_mask"] = "mask"
	d.Values["output"] = "output"
	params, err = n.embeddingParams(d)
	if err != nil {
		t.Fatal(err)
	}
	json.Unmarshal(params, &obj)
	profile := obj["profile"].(map[string]any)
	if profile["endpoint"] != nil || profile["local"].(map[string]any)["tensors"].(map[string]any)["token_type_ids"] != nil {
		t.Fatal("local profile mixed API settings")
	}
	d.Values["dimensions"] = "0"
	if _, err = n.embeddingParams(d); err == nil {
		t.Fatal("invalid dimensions admitted")
	}
}
