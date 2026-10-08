package main

import (
	"context"
	"encoding/json"
	"sync"
	"testing"
	"time"

	"github.com/bloodf/beans/desktop/model"
)

func TestUpdateWorkerAdmissionOrdersWithNativeHostTransitions(t *testing.T) {
	previous, old := executeMain, native
	executor := make(chan func(), 16)
	stopped := make(chan struct{})
	go func() {
		defer close(stopped)
		for fn := range executor {
			fn()
		}
	}()
	executeMain = func(fn func()) { done := make(chan struct{}); executor <- func() { fn(); close(done) }; <-done }
	native = &nativeDesktop{store: model.NewNativeStore()}
	defer func() {
		barrier := make(chan struct{})
		postMain(func() { close(barrier) })
		<-barrier
		// Wait for the dispatcher to finish using the executor before restoring it.
		for {
			mainPosts.Lock()
			idle := !mainPosts.running
			mainPosts.Unlock()
			if idle {
				break
			}
			time.Sleep(time.Millisecond)
		}
		executeMain = previous
		native = old
		close(executor)
		<-stopped
	}()
	host := &appDelegate{cli: newCLIClient(), session: model.NewSession(func(bool) {}, func(string, json.RawMessage) {})}
	var workers sync.WaitGroup
	for i := 0; i < 100; i++ {
		postMain(func() { native.store.Draft("c").Text = "private"; host.connectionTransition("disconnected") })
		workers.Add(1)
		go func() {
			defer workers.Done()
			ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
			defer cancel()
			if _, err := desktopUpdateIdle(ctx, nil); err == nil {
				t.Error("worker admitted native intent")
			}
			if err := (&desktopUpdateLease{}).guardPages(ctx); err == nil {
				t.Error("lease admitted native intent")
			}
		}()
		postMain(func() { native.store.Reset(); native.store.Draft("c").ReplyTo = "reply" })
	}
	workers.Wait()
}
