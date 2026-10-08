package main

import (
	"encoding/json"
	"testing"

	"github.com/bloodf/beans/desktop/model"
)

func TestHostReconnectPreservesMemorySecretWithoutCrossAccountDisplay(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	old := native
	defer func() { native = old }()
	native = &nativeDesktop{store: model.NewNativeStore(), memory: &nativeMemory{runtime: r}}
	native.store.AccountID = "a"
	native.store.Connected = true
	secret := &memoryConnectionForm{Name: "Unsaved", SecretMode: "replace", Secret: " exact secret "}
	native.memoryPanel = nativeMemoryPanel{Open: true, Loading: true, Forms: memoryForms{Connection: secret, Pending: true}}
	host := &appDelegate{cli: newCLIClient(), session: model.NewSession(func(bool) {}, func(string, json.RawMessage) {})}
	host.connectionTransition("disconnected")
	host.connectionTransition("connected")
	if native.memoryPanel.Open || native.memoryPanel.Loading || native.memoryRecovery["a"].Connection != secret {
		t.Fatal("transition discarded secret or displayed stale panel")
	}
	native.event("snapshot", json.RawMessage(`{"has_identity":true,"identity_id":"b","chats":[]}`))
	if native.memoryPanel.Forms.Connection != nil || native.memoryRecovery["a"].Connection.Secret != " exact secret " {
		t.Fatal("old draft crossed account or disappeared")
	}
	native.event("snapshot", json.RawMessage(`{"has_identity":true,"identity_id":"a","chats":[]}`))
	if native.memoryPanel.Forms.Connection != secret || native.memoryPanel.Forms.Pending || !native.memoryPanel.Open {
		t.Fatal("same-account draft not recovered")
	}
}
func TestSuspendMemoryConsentDropsApprovalsNotChoices(t *testing.T) {
	n := &nativeDesktop{store: model.NewNativeStore()}
	n.store.AccountID = "a"
	b := &memoryBotForm{Connection: "c", Capture: true, Group: true, Cap: "3", Draft: 5, View: 6, Approval: 7, Confirm: true, DeleteConfirm: true, Health: json.RawMessage(`{}`)}
	n.memoryPanel.Forms.Bot = b
	n.suspendMemoryForms()
	if !b.Capture || !b.Group || b.Cap != "3" || b.Connection != "c" {
		t.Fatal("consent choices discarded")
	}
	if b.Draft != 0 || b.View != 0 || b.Approval != 0 || b.Confirm || b.DeleteConfirm || b.Health != nil || !b.Recovering {
		t.Fatal("old authority survived")
	}
	if !n.hasMemoryIntent() {
		t.Fatal("archived form bypassed quit ownership")
	}
}
