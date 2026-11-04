package main

import (
	"testing"

	"github.com/egoist/mygo"
)

func TestNativeWindowTitleDefersAndFencesUpdates(t *testing.T) {
	oldPost, oldSet := postNativeWindowTitle, setNativeWindowTitle
	t.Cleanup(func() { postNativeWindowTitle, setNativeWindowTitle = oldPost, oldSet })
	var queued []func()
	postNativeWindowTitle = func(fn func()) { queued = append(queued, fn) }
	type update struct {
		win   *mygo.Window
		title string
	}
	var applied []update
	setNativeWindowTitle = func(win *mygo.Window, title string) { applied = append(applied, update{win, title}) }
	drain := func() {
		batch := queued
		queued = nil
		for _, fn := range batch {
			fn()
		}
	}
	first, second := &mygo.Window{}, &mygo.Window{}
	n := &nativeDesktop{win: first}
	n.queueWindowTitle("First")
	n.queueWindowTitle("First")
	if len(queued) != 1 || len(applied) != 0 {
		t.Fatal("title was not deferred and deduplicated")
	}
	n.queueWindowTitle("Latest")
	n.queueWindowTitle("First")
	n.queueWindowTitle("Latest")
	drain()
	if len(applied) != 1 || applied[0] != (update{first, "Latest"}) {
		t.Fatal("superseded title was applied")
	}
	n.queueWindowTitle("Closed")
	n.win = nil
	drain()
	if len(applied) != 1 {
		t.Fatal("closed window received a title")
	}
	n.win = second
	n.queueWindowTitle("Latest")
	drain()
	if len(applied) != 2 || applied[1] != (update{second, "Latest"}) {
		t.Fatal("reopened window inherited the previous window's title cache")
	}
	n.queueWindowTitle("Stale")
	n.win = first
	n.queueWindowTitle("Replacement")
	drain()
	if len(applied) != 3 || applied[2] != (update{first, "Replacement"}) {
		t.Fatal("replaced window received a stale title")
	}
}
