package main

import (
	"encoding/json"
	"testing"
	"time"

	"github.com/bloodf/beans/desktop/model"
)

func TestNativeConnectionFailureRetainsSecretAndBlocksCancelWhilePending(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	oldExecutor := executeMain
	queue := make(chan func(), 8)
	executeMain = func(fn func()) { queue <- fn }
	defer func() { executeMain = oldExecutor }()
	n := &nativeDesktop{store: model.NewNativeStore(), memory: &nativeMemory{runtime: r}}
	n.store.Connected = true
	n.editMemoryConnection(nil)
	d := n.memoryPanel.Forms.Connection
	d.Name = "Fixture"
	d.SecretMode = "replace"
	d.Secret = " exact secret "
	t.Setenv("BEANS_MOCK", "1")
	n.saveMemoryConnection()
	if !n.memoryPanel.Forms.Pending {
		t.Fatal("mutation did not enter pending")
	}
	select {
	case fn := <-queue:
		fn()
	case <-time.After(5 * time.Second):
		t.Fatal("completion not posted")
	}
	if n.memoryPanel.Forms.Pending || n.memoryPanel.Forms.Connection != d || d.Secret != " exact secret " || n.memoryPanel.Forms.Error == "" {
		t.Fatal("failure discarded connection draft")
	}
	// Wait for queue drain before restoring the executor.
	for {
		mainPosts.Lock()
		idle := !mainPosts.running
		mainPosts.Unlock()
		if idle {
			break
		}
		time.Sleep(time.Millisecond)
	}
}
func TestNativeConsentControlsUseExactSharedDraft(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	n := &nativeDesktop{memory: &nativeMemory{runtime: r}}
	id, err := n.memory.newDraft("bot", nil)
	if err != nil {
		t.Fatal(err)
	}
	b := &memoryBotForm{ID: "bot", Connection: "connection", Capture: true, Cap: "1", Timeout: "2000", Bytes: "16384", Results: "8", Context: "4000", Draft: id}
	n.memoryPanel.Forms.Bot = b
	if err = n.syncMemoryConsent(); err != nil {
		t.Fatal(err)
	}
	if _, err = n.memory.draft(id, "request", nil); err == nil {
		t.Fatal("controls bypassed consent")
	}
	n.memory.draft(id, "approvePlaintext", nil)
	b.Cap = "2"
	n.syncMemoryConsent()
	if _, err = n.memory.draft(id, "request", nil); err == nil {
		t.Fatal("cap extension reused approval")
	}
	n.memory.draft(id, "approvePlaintext", nil)
	request, err := n.memory.draft(id, "request", nil)
	if err != nil {
		t.Fatal(err)
	}
	var rpc struct {
		Params struct {
			Cap int `json:"max_capture_deliveries_per_turn"`
		} `json:"params"`
	}
	if json.Unmarshal(request, &rpc) != nil || rpc.Params.Cap != 2 {
		t.Fatal("exact draft not serialized")
	}
}
