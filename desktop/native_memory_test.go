package main

import (
	"context"
	"encoding/json"
	"strings"
	"testing"
)

func TestNativeSetupApprovalIsOneUseAndEpochBound(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := &nativeMemory{runtime: r}
	preview := json.RawMessage(`{"action":"memory.lance.binding.preview","token":"token","expires_at":1000,"runner_id":"r","bot_id":"b","connection_revision":{"counter":1,"device_id":"d"},"profile_id":null,"profile_revision":null,"details":{"directory":"/fixture","create":false,"table":"beans_memory_v1","namespace":"` + strings.Repeat("a", 64) + `"}}`)
	id, err := m.setupApproval("memory.lance.binding.preview", preview, 0)
	if err != nil {
		t.Fatal(err)
	}
	target := json.RawMessage(`{"runner_id":"r","bot_id":"b","connection_revision":{"counter":1,"device_id":"d"}}`)
	if _, err = m.setupApply(id, false, target, 1); err == nil {
		t.Fatal("unconfirmed apply admitted")
	}
	request, err := m.setupApply(id, true, target, 1)
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(request), "directory") {
		t.Fatal("apply reauthored setup")
	}
	if _, err = m.setupApply(id, true, target, 2); err == nil {
		t.Fatal("approval reused")
	}
	id, err = m.setupApproval("memory.lance.binding.preview", preview, 0)
	if err != nil {
		t.Fatal(err)
	}
	m.reset(2)
	if _, err = m.setupApply(id, true, target, 3); err == nil {
		t.Fatal("approval crossed epoch")
	}
}
func TestNativeMemoryRejectsUnsupportedAndDemoBeforeTransport(t *testing.T) {
	n := &nativeDesktop{}
	if _, err := n.memoryRequest(context.Background(), "memory.lance.initialize", nil, cliAuthority{}); err == nil {
		t.Fatal("superseded alias admitted")
	}
	t.Setenv("BEANS_MOCK", "1")
	if _, err := n.memoryRequest(context.Background(), "memory.connections.list", nil, cliAuthority{}); err == nil {
		t.Fatal("demo sent memory RPC")
	}
}

func TestNativeDeletionRefreshDoesNotUnlockStaleEpoch(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := &nativeMemory{runtime: r}
	prefs := json.RawMessage(`{"connection_id":null,"auto_recall":false,"capture_conversation":false,"capture_group_text":false,"unattended_capture":false,"max_capture_deliveries_per_turn":1,"recall_budget":{"timeout_ms":2000,"max_bytes":16384,"max_results":8,"max_context_chars":4000},"consent_revision":{"counter":1,"device_id":"d"},"deletion_epoch":3}`)
	id, err := m.newView("b", prefs)
	if err != nil {
		t.Fatal(err)
	}
	if _, err = m.deletion(id, "begin", nil); err != nil {
		t.Fatal(err)
	}
	m.deletion(id, "record", 4)
	if _, err = m.deletion(id, "refresh", prefs); err == nil {
		t.Fatal("stale epoch unlocked deletion")
	}
	locked, err := m.deletion(id, "locked", nil)
	if err != nil || string(locked) != "true" {
		t.Fatalf("deletion lock: %s %v", locked, err)
	}
}

func TestNativeMemoryAdvancedRequiresExactNegotiatedVerb(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := &nativeMemory{runtime: r}
	for _, caps := range []json.RawMessage{json.RawMessage(`{}`), json.RawMessage(`{"advanced":["memory_edit"]}`), json.RawMessage(`{"advanced":["memory_edit"],"advanced_actions":{"memory_edit":["get"]}}`)} {
		ok, err := m.supportsAdvanced(caps, "memory_edit", "edit")
		if err != nil || ok {
			t.Fatalf("unnegotiated edit admitted: %v %v", ok, err)
		}
	}
	ok, err := m.supportsAdvanced(json.RawMessage(`{"advanced":["memory_edit"],"advanced_actions":{"memory_edit":["edit"]}}`), "memory_edit", "edit")
	if err != nil || !ok {
		t.Fatal("negotiated edit unavailable")
	}
}
