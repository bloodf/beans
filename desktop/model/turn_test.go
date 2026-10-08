package model

import (
	"encoding/json"
	"testing"
)

func TestNativeTurnsDoNotSurviveDisconnectOrIdentityLoss(t *testing.T) {
	s := NewNativeStore()
	s.Apply("snapshot", json.RawMessage(`{"has_identity":true,"identity_id":"account","running_turns":[{"job_id":"j1","chat_id":"c"},{"job_id":"j2","chat_id":"c"}]}`))
	s.Apply("job.finished", json.RawMessage(`{"job_id":"j1","chat_id":"c"}`))
	if _, ok := s.Running["j1"]; ok {
		t.Fatal("finished turn remained")
	}
	if s.Running["j2"] != "c" {
		t.Fatal("another turn was cleared")
	}
	s.Fence()
	if len(s.Running) != 0 {
		t.Fatal("disconnected turns remained")
	}
	s.Apply("job.started", json.RawMessage(`{"job_id":"j3","chat_id":"c"}`))
	s.Apply("identity.changed", json.RawMessage(`{"has_identity":false}`))
	s.Apply("job.started", json.RawMessage(`{"job_id":"old","chat_id":"c"}`))
	if len(s.Running) != 0 {
		t.Fatal("old account turn restored")
	}
}
