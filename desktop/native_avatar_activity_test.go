package main

import (
	"encoding/json"
	"strings"
	"testing"
	"time"
)

func TestNativeAvatarActivityConsumer(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	now := time.UnixMilli(10000)
	invalidations := 0
	a := newNativeAvatarActivity(r, func() { invalidations++ })
	a.now = func() time.Time { return now }
	var fire func()
	var queued []func()
	a.after = func(delay time.Duration, fn func()) *time.Timer {
		if delay != 5*time.Second {
			t.Fatalf("error expiry delay: %v", delay)
		}
		fire = fn
		return nil
	}
	a.post = func(fn func()) { queued = append(queued, fn) }
	event := func(name, data string) {
		t.Helper()
		if err := a.event(name, json.RawMessage(data)); err != nil {
			t.Fatal(err)
		}
	}
	state := func(chat, want string) {
		t.Helper()
		if got := a.state("b", chat); got != want {
			t.Fatalf("chat %q: got %q, want %q", chat, got, want)
		}
	}
	snapshot := `{"chats":[{"id":"c1","bot_ids":["b"]},{"id":"c2","bot_ids":["b"]}],"running_turns":[]}`
	start := `{"job_id":"j1","chat_id":"c1","bot_id":"b"}`
	failure := `{"chat_id":"c1","message":{"id":"m","chat_id":"c1","author":{"kind":"bot","bot_id":"b"},"body":{"kind":"text","text":"failed"},"state":{"kind":"failed"},"created_at":10}}`
	event("snapshot", snapshot)
	event("job.started", start)
	event("job.thinking", start)
	state("c1", "thinking")
	event("job.started", `{"job_id":"j2","chat_id":"c2","bot_id":"b"}`)
	event("job.retry", `{"chat_id":"c2","bot_id":"b"}`)
	state("c2", "retry")
	state("", "retry")
	event("message.added", failure)
	event("job.finished", start)
	state("c1", "error")
	state("", "error")
	before := invalidations
	fire()
	if invalidations != before {
		t.Fatal("timer mutated state before returning to main queue")
	}
	now = now.Add(5 * time.Second)
	queued[0]()
	state("c1", "idle")
	state("", "retry")
	if invalidations != before+1 {
		t.Fatal("expiry did not invalidate consumer")
	}

	// A queued old expiry cannot refresh a newer event's state or repaint it.
	event("job.started", start)
	event("message.added", strings.Replace(failure, `"id":"m"`, `"id":"m2"`, 1))
	fire()
	stale := queued[len(queued)-1]
	event("snapshot", snapshot)
	before = invalidations
	stale()
	state("", "idle")
	if invalidations != before {
		t.Fatal("snapshot retained queued expiry authority")
	}
	event("job.started", start)
	event("message.added", strings.Replace(failure, `"id":"m"`, `"id":"m3"`, 1))
	fire()
	stale = queued[len(queued)-1]
	a.reset()
	before = invalidations
	stale()
	state("c1", "idle")
	state("", "idle")
	if invalidations != before {
		t.Fatal("reset retained queued expiry authority")
	}
	// Reset clears the shared global, not only the Go cache.
	event("unrelated", `{}`)
	state("", "idle")
	if a.state("missing", "missing") != "idle" || (*nativeAvatarActivity)(nil).state("b", "") != "idle" {
		t.Fatal("missing portraits must be idle")
	}
}
