package model

import (
	"encoding/json"
	"errors"
	"testing"
)

func TestNativeSendAccountFence(t *testing.T) {
	s := NewNativeStore()
	s.Connected = true
	snapshot := json.RawMessage(`{"has_identity":true,"identity_id":"account","bots":[{"id":"b","name":"Bot","look":{"version":1,"base":{},"future":true}}],"chats":[{"id":"c","bot_ids":["b"],"messages":[]}]}`)
	if err := s.Apply("snapshot", snapshot); err != nil {
		t.Fatal(err)
	}
	d := s.Draft("c")
	d.Text = "first"
	_, finish, err := s.BeginSend("c", "m")
	if err != nil {
		t.Fatal(err)
	}
	d.Text = "next"
	d.Revision++
	finish(nil)
	if d.Text != "next" || d.Sending {
		t.Fatal("completion erased next draft")
	}
	_, finish, err = s.BeginSend("c", "m2")
	if err != nil {
		t.Fatal(err)
	}
	if err := s.Apply("identity.changed", json.RawMessage(`{"has_identity":false}`)); err != nil {
		t.Fatal(err)
	}
	s.Apply("roster.changed", snapshot)
	if len(s.Chats) != 0 {
		t.Fatal("post-teardown roster restored account")
	}
	s.Apply("snapshot", snapshot)
	next := s.Draft("c")
	next.Text = "new account"
	finish(errors.New("old account error"))
	if next.Error != "" || next.Text != "new account" {
		t.Fatal("old completion changed new account")
	}
	if string(s.Bots[0].Look) != `{"version":1,"base":{},"future":true}` {
		t.Fatal("appearance fields lost")
	}
	next.Text = ""
	next.Attachments = []NativeAttachment{{ID: "a", Path: "/fixture", Name: "fixture.txt"}}
	next.ReplyTo = "quoted"
	params, done, err := s.BeginSend("c", "attachment-message")
	if err != nil {
		t.Fatal(err)
	}
	if params["reply_to"] != "quoted" {
		t.Fatal("reply omitted")
	}
	done(errors.New("send refused"))
	if next.Error != "send refused" || next.ReplyTo != "quoted" || next.Attachments[0].ID != "a" {
		t.Fatal("failure discarded intent")
	}
	_, done, err = s.BeginSend("c", "retry-message")
	if err != nil {
		t.Fatal(err)
	}
	done(nil)
	if next.ReplyTo != "" || len(next.Attachments) != 0 {
		t.Fatal("successful send retained sent intent")
	}
}
