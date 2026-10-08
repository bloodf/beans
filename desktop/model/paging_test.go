package model

import (
	"encoding/json"
	"testing"
)

func TestNativeOlderPageRejectsAccountReplacement(t *testing.T) {
	s := NewNativeStore()
	s.Connected = true
	snapshot := json.RawMessage(`{"has_identity":true,"identity_id":"account","chats":[{"id":"c","has_more":true,"messages":[{"id":"newest"}]}]}`)
	s.Apply("snapshot", snapshot)
	_, done, err := s.BeginOlder("c")
	if err != nil {
		t.Fatal(err)
	}
	done(json.RawMessage(`{"has_more":false,"messages":[{"id":"older"},{"id":"newest"}]}`), nil)
	c := s.Chat("c")
	if c.HasMore || len(c.Messages) != 2 || c.Messages[0].ID != "older" || c.Messages[1].ID != "newest" {
		t.Fatal("page order or deduplication failed")
	}
	s.Apply("snapshot", snapshot)
	_, done, err = s.BeginOlder("c")
	if err != nil {
		t.Fatal(err)
	}
	s.Reset()
	s.Apply("snapshot", snapshot)
	done(json.RawMessage(`{"messages":[{"id":"foreign"}]}`), nil)
	if s.Chat("c").Messages[0].ID != "newest" {
		t.Fatal("old page crossed account fence")
	}
}
