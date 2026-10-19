package model

import (
	"encoding/json"
	"testing"
)

func TestBootstrapOrdersEventsAndRejectsPreviousConnection(t *testing.T) {
	var identities []bool
	var seen []string
	s := NewSession(func(has bool) { identities = append(identities, has) }, func(name string, data json.RawMessage) { seen = append(seen, name) })
	old := s.Connect()
	s.Event(old, "message.added", json.RawMessage(`{"id":"old"}`))
	current := s.Connect()
	s.Event(current, "message.added", json.RawMessage(`{"id":"current"}`))
	s.Bootstrap(old, json.RawMessage(`{"has_identity":false}`))
	if len(identities) != 0 || len(seen) != 0 {
		t.Fatal("stale bootstrap published state")
	}
	if err := s.Bootstrap(current, json.RawMessage(`{"has_identity":true}`)); err != nil {
		t.Fatal(err)
	}
	if len(identities) != 1 || !identities[0] {
		t.Fatalf("identities: %v", identities)
	}
	if len(seen) != 2 || seen[0] != "snapshot" || seen[1] != "message.added" {
		t.Fatalf("event order: %v", seen)
	}
	s.Disconnect()
	s.Event(current, "identity.changed", json.RawMessage(`{"has_identity":false}`))
	if len(identities) != 1 {
		t.Fatal("disconnected event changed identity")
	}
}

func TestIdentityLossBypassesBootstrapAndInvalidatesCompletion(t *testing.T) {
	var identities []bool
	var events []string
	s := NewSession(func(has bool) { identities = append(identities, has) }, func(name string, data json.RawMessage) { events = append(events, name) })
	generation := s.Connect()
	s.Event(generation, "identity.changed", json.RawMessage(`{"has_identity":false}`))
	if len(identities) != 1 || identities[0] {
		t.Fatalf("teardown delayed: %v", identities)
	}
	if err := s.Bootstrap(generation, json.RawMessage(`{"has_identity":true}`)); err != nil {
		t.Fatal(err)
	}
	if len(identities) != 1 || len(events) != 1 {
		t.Fatal("pre-teardown completion restored account")
	}
}

func TestMalformedBootstrapCannotAdmitAccount(t *testing.T) {
	for _, data := range []string{`{}`, `{"has_identity":null}`, `{"has_identity":"true"}`, `not json`} {
		s := NewSession(func(bool) { t.Fatal("invalid identity admitted") }, func(string, json.RawMessage) { t.Fatal("invalid snapshot published") })
		if err := s.Bootstrap(s.Connect(), json.RawMessage(data)); err == nil {
			t.Fatalf("accepted %s", data)
		}
	}
}
