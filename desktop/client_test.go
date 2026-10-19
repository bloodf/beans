package main

import (
	"encoding/json/jsontext"
	"testing"
)

func TestClientRejectsFramesFromPreviousConnection(t *testing.T) {
	c := newCLIClient()
	c.generation = 2
	c.state = "connected"
	var events []string
	c.onEvent = func(name string, frame []byte) { events = append(events, name) }
	waiting := make(chan reply, 1)
	c.pending[1] = waiting
	c.handle(1, []byte(`{"event":"identity.changed","data":{"has_identity":false}}`))
	c.handle(1, []byte(`{"id":1,"result":{"old":true}}`))
	if len(events) != 0 {
		t.Fatalf("old connection delivered events: %v", events)
	}
	select {
	case <-waiting:
		t.Fatal("old connection completed current request")
	default:
	}
	c.handle(2, []byte(`{"id":1,"result":{"current":true}}`))
	if got := <-waiting; string(got.result) != `{"current":true}` || got.err != nil {
		t.Fatalf("current reply: %+v", got)
	}
	c.handle(2, []byte(`{"event":"identity.changed","data":{"has_identity":true}}`))
	if len(events) != 1 || events[0] != "identity.changed" {
		t.Fatalf("current events: %v", events)
	}
}

func TestClientInvalidatesPendingRepliesOnDrop(t *testing.T) {
	c := newCLIClient()
	c.generation = 3
	c.state = "connected"
	waiting := make(chan reply, 1)
	c.pending[4] = waiting
	c.dropped(2)
	if len(c.pending) != 1 {
		t.Fatal("stale drop removed current request")
	}
	c.dropped(3)
	if answer := <-waiting; answer.err != errConnectionDrop || len(answer.result) != 0 {
		t.Fatalf("drop reply: %+v", answer)
	}
	c.handle(3, []byte(`{"id":4,"result":null}`))
	select {
	case answer := <-waiting:
		t.Fatalf("late reply: %s", jsontext.Value(answer.result))
	default:
	}
}
