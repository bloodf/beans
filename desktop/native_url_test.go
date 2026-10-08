package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/bloodf/beans/desktop/model"
)

func TestGojaLocalAssetPlansValidateHTTPSWithoutBrowser(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := &nativeMemory{runtime: r}
	for _, source := range []struct {
		url   string
		valid bool
	}{{"https://assets.example/model.bin", true}, {"http://assets.example/model.bin", false}, {"https://user:password@assets.example/model.bin", false}, {"https://assets.example/model.bin#fragment", false}, {"https://assets.example/model.bin?version=1", true}, {"https://", false}} {
		assets := []any{}
		for _, kind := range []string{"runtime", "model", "tokenizer"} {
			assets = append(assets, map[string]any{"kind": kind, "source": map[string]string{"kind": "download", "url": source.url}, "license": "MIT", "bytes": 1, "sha256": strings.Repeat("a", 64)})
		}
		request := jsonBytes(map[string]any{"method": "memory.embeddings.local.preview", "params": map[string]any{"runner_id": "r", "profile_id": "p", "profile_revision": map[string]any{"counter": 1, "device_id": "d"}, "plan": map[string]any{"assets": assets}}})
		out, err := m.setupPreview(request)
		if source.valid {
			if err != nil {
				t.Fatalf("valid HTTPS plan refused: %v", err)
			}
			if !strings.Contains(string(out), source.url) {
				t.Fatal("exact approved URL changed")
			}
		} else if err == nil {
			t.Fatalf("unsafe source admitted: %s", source.url)
		}
	}
}
func TestNativePanelReconnectClearsPendingAndDemoDoesNotDispatch(t *testing.T) {
	old := native
	defer func() { native = old }()
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	native = &nativeDesktop{store: model.NewNativeStore(), memory: &nativeMemory{runtime: r}, memoryPanel: nativeMemoryPanel{Open: true, Loading: true, Connections: json.RawMessage(`{"stale":true}`)}}
	host := &appDelegate{cli: newCLIClient(), session: model.NewSession(func(bool) {}, func(string, json.RawMessage) {})}
	host.connectionTransition("connected")
	if native.memoryPanel.Loading || native.memoryPanel.Open || native.memoryPanel.Connections != nil {
		t.Fatal("reconnect retained stale panel")
	}
	native.store.Connected = true
	t.Setenv("BEANS_MOCK", "1")
	native.loadMemory()
	if native.memoryPanel.Loading || native.memoryPanel.Error == "" {
		t.Fatal("demo panel did not refuse before dispatch")
	}
}
