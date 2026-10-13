package main

import (
	"encoding/json"
	"time"
)

// Activity is evaluated once per admitted event, not per portrait or paint.
// All methods and queued expiry completions belong to the ordered main queue.
type nativeAvatarActivity struct {
	runtime    *sharedRuntime
	chats      map[string]map[string]string
	bots       map[string]string
	timer      *time.Timer
	generation uint64
	now        func() time.Time
	after      func(time.Duration, func()) *time.Timer
	post       func(func())
	invalidate func()
}

func newNativeAvatarActivity(r *sharedRuntime, invalidate func()) *nativeAvatarActivity {
	return &nativeAvatarActivity{runtime: r, now: time.Now, after: time.AfterFunc, post: postMain, invalidate: invalidate}
}

func (a *nativeAvatarActivity) reset() {
	a.generation++
	if a.timer != nil {
		a.timer.Stop()
		a.timer = nil
	}
	a.chats = nil
	a.bots = nil
	var ignored any
	_ = a.runtime.eval(`nativeAvatarActivity.reset();return null;`, nil, &ignored)
	if a.invalidate != nil {
		a.invalidate()
	}
}

func (a *nativeAvatarActivity) event(name string, data json.RawMessage) error {
	var ignored any
	if err := a.runtime.eval(`nativeAvatarActivity.event(input.name,input.data,input.now);return null;`, map[string]any{"name": name, "data": data, "now": a.now().UnixMilli()}, &ignored); err != nil {
		return err
	}
	if err := a.refresh(); err != nil {
		return err
	}
	if a.invalidate != nil {
		a.invalidate()
	}
	return nil
}

func (a *nativeAvatarActivity) refresh() error {
	var out struct {
		Chats   map[string]map[string]string `json:"chats"`
		Bots    map[string]string            `json:"bots"`
		Expires int64                        `json:"expires"`
	}
	now := a.now().UnixMilli()
	if err := a.runtime.eval(`return nativeAvatarActivity.states(input);`, now, &out); err != nil {
		return err
	}
	a.chats = out.Chats
	a.bots = out.Bots
	a.generation++
	generation := a.generation
	if a.timer != nil {
		a.timer.Stop()
		a.timer = nil
	}
	if out.Expires > now {
		a.timer = a.after(time.Duration(out.Expires-now)*time.Millisecond, func() {
			a.post(func() {
				if a.generation != generation {
					return
				}
				if err := a.refresh(); err == nil && a.invalidate != nil {
					a.invalidate()
				}
			})
		})
	}
	return nil
}

func (a *nativeAvatarActivity) state(botID, chatID string) string {
	if a == nil {
		return "idle"
	}
	state := a.bots[botID]
	if chatID != "" {
		state = a.chats[chatID][botID]
	}
	if state == "" {
		return "idle"
	}
	return state
}
