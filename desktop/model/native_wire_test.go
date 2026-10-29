package model

import (
	"encoding/json"
	"testing"
)

func TestWireToBotKeepsBeansCapabilitiesAndLook(t *testing.T) {
	wire := decodeJSON[WireBot](t, `{
		"id": "bot-1",
		"name": "Bot",
		"description": "",
		"symbol_name": "sparkles",
		"accent": "teal",
		"runner_id": "r1",
		"provider": "anthropic",
		"model": "claude",
		"thinking": "low",
		"capabilities": {"shell": false, "write": true, "plugins": []},
		"look": {"version": 1, "base": {"expression": "happy"}},
		"created_at": 1,
		"shape": "ignored-upstream-character",
		"permissions": {"shell": false}
	}`)
	bot := ToBot(wire)
	if bot.SymbolName != "sparkles" || bot.Accent != "teal" || bot.Provider != "anthropic" {
		t.Fatalf("core fields: %+v", bot)
	}
	if bot.Capabilities.Shell || !bot.Capabilities.Write || bot.Capabilities.Plugins == nil || len(*bot.Capabilities.Plugins) != 0 {
		t.Fatalf("capabilities: %+v", bot.Capabilities)
	}
	if string(bot.Look) != `{"version": 1, "base": {"expression": "happy"}}` {
		t.Fatalf("look lost: %s", bot.Look)
	}
}

func TestWireSearchResultsAcceptBeansFiles(t *testing.T) {
	results := decodeJSON[WireSearchResults](t, `{
		"chats": [{"chat_id": "c1", "snippet": "hello"}],
		"messages": [{"chat_id": "c1", "message_id": "m1", "snippet": "hi", "author": {"kind": "you"}, "created_at": 1}],
		"files": [{"chat_id": "c1", "message_id": "m1", "attachment_id": "a1", "name": "note.txt", "snippet": "note", "created_at": 1}],
		"history_complete": true,
		"agents": [{"id": "unsupported"}]
	}`)
	if len(results.Chats) != 1 || len(results.Messages) != 1 || len(results.Files) != 1 || !results.HistoryComplete {
		t.Fatalf("search decode: %+v", results)
	}
}

func TestWireToMessageKeepsReplyQuote(t *testing.T) {
	msg := ToMessage(decodeJSON[WireMessage](t, `{
		"id": "m2",
		"chat_id": "c1",
		"author": {"kind": "you"},
		"body": {
			"kind": "text",
			"text": "follow up",
			"reply_to": {"message_id": "m1", "author": {"kind": "bot", "bot_id": "b1"}, "text": "earlier"}
		},
		"state": {"kind": "complete"},
		"created_at": 2
	}`))
	if msg.ReplyTo == nil || msg.ReplyTo.MessageID != "m1" || msg.ReplyTo.Text != "earlier" || msg.ReplyTo.Author.BotID != "b1" {
		t.Fatalf("reply: %+v", msg.ReplyTo)
	}
}

func TestNativeStoreMessageOrderingAndUnread(t *testing.T) {
	s := NewNativeStore()
	s.Connected = true
	snapshot := json.RawMessage(`{
		"has_identity": true,
		"identity_id": "account",
		"bots": [{"id": "b", "name": "Bot", "look": {"version": 1}}],
		"chats": [{"id": "c", "bot_ids": ["b"], "unread_count": 2, "messages": [
			{"id": "m1", "chat_id": "c", "author": {"kind": "bot", "bot_id": "b"}, "body": {"kind": "text", "text": "one"}, "state": {"kind": "complete"}, "created_at": 1},
			{"id": "m2", "chat_id": "c", "author": {"kind": "you"}, "body": {"kind": "text", "text": "two"}, "state": {"kind": "complete"}, "created_at": 2}
		]}]
	}`)
	if err := s.Apply("snapshot", snapshot); err != nil {
		t.Fatal(err)
	}
	chat := s.Chat("c")
	if chat == nil || len(chat.Messages) != 2 || chat.Messages[0].ID != "m1" || chat.Messages[1].ID != "m2" {
		t.Fatalf("order: %+v", chat)
	}
	// Snapshot selection marks the first chat read.
	if chat.UnreadCount != 0 {
		t.Fatalf("selected chat still unread: %d", chat.UnreadCount)
	}
	s.Selected = ""
	chat.UnreadCount = 2
	if err := s.Apply("message.added", json.RawMessage(`{
		"chat_id": "c",
		"message": {
			"id": "m3",
			"chat_id": "c",
			"author": {"kind": "bot", "bot_id": "b"},
			"body": {"kind": "text", "text": "**three**", "reply_to": {"message_id": "m2", "author": {"kind": "you"}, "text": "two"}},
			"state": {"kind": "complete"},
			"created_at": 3
		}
	}`)); err != nil {
		t.Fatal(err)
	}
	chat = s.Chat("c")
	if len(chat.Messages) != 3 || chat.Messages[2].ID != "m3" {
		t.Fatalf("added: count=%d last=%+v", len(chat.Messages), chat.Messages[len(chat.Messages)-1])
	}
	// Unread stays at the CLI-supplied count; message.added never bumps locally.
	if chat.UnreadCount != 2 {
		t.Fatalf("message.added bumped unread to %d", chat.UnreadCount)
	}
	if quote := chat.Messages[2].Body.ReplyTo; quote == nil || quote.MessageID != "m2" || quote.Text != "two" {
		t.Fatalf("reply quote: %+v", quote)
	}
	if got := chat.Messages[2].DisplayText(); got != "three" {
		t.Fatalf("markdown display: %q", got)
	}
	s.SelectChat("c")
	if s.Selected != "c" || chat.UnreadCount != 0 {
		t.Fatalf("select/mark read: selected=%q unread=%d", s.Selected, chat.UnreadCount)
	}
}

func TestNativeStoreMalformedPayloadLeavesState(t *testing.T) {
	s := NewNativeStore()
	s.Connected = true
	good := json.RawMessage(`{"has_identity":true,"identity_id":"account","bots":[{"id":"b","name":"Bot"}],"chats":[{"id":"c","bot_ids":["b"],"unread_count":3,"messages":[{"id":"m1","chat_id":"c","author":{"kind":"you"},"body":{"kind":"text","text":"hi"},"state":{"kind":"complete"},"created_at":1}]}]}`)
	if err := s.Apply("snapshot", good); err != nil {
		t.Fatal(err)
	}
	beforeBots, beforeChats, beforeUnread := len(s.Bots), len(s.Chats), s.Chat("c").UnreadCount
	if err := s.Apply("roster.changed", json.RawMessage(`{"bots":[`)); err == nil {
		t.Fatal("expected malformed roster error")
	}
	if len(s.Bots) != beforeBots || len(s.Chats) != beforeChats || s.Chat("c").UnreadCount != beforeUnread {
		t.Fatal("malformed roster corrupted store")
	}
	if err := s.Apply("message.added", json.RawMessage(`{"chat_id":"c","message":{"id":""}}`)); err == nil {
		t.Fatal("expected missing message id error")
	}
	if len(s.Chat("c").Messages) != 1 {
		t.Fatal("bad message mutated transcript")
	}
	// Unsupported event names and unknown fields must be ignored.
	if err := s.Apply("coding.agents.changed", json.RawMessage(`{"agents":[{"id":"x"}]}`)); err != nil {
		t.Fatal(err)
	}
	if err := s.Apply("message.updated", json.RawMessage(`{
		"chat_id": "c",
		"message": {
			"id": "m1",
			"chat_id": "c",
			"author": {"kind": "you"},
			"body": {"kind": "text", "text": "hi again", "agent_card": {"id": "unsupported"}},
			"state": {"kind": "complete"},
			"created_at": 1,
			"character_clip": true
		}
	}`)); err != nil {
		t.Fatal(err)
	}
	msg := s.Chat("c").Messages[0]
	if msg.Body.Text != "hi again" || len(s.Chats) != 1 || s.AccountID != "account" {
		t.Fatalf("unsupported fields corrupted state: %+v", msg)
	}
}

func TestNativeStoreRosterPreservesUnreadWithoutWireCount(t *testing.T) {
	s := NewNativeStore()
	s.Connected = true
	if err := s.Apply("snapshot", json.RawMessage(`{"has_identity":true,"identity_id":"account","bots":[{"id":"b","name":"Bot"}],"chats":[{"id":"c","bot_ids":["b"],"messages":[]}]}`)); err != nil {
		t.Fatal(err)
	}
	s.Selected = ""
	s.Chat("c").UnreadCount = 4
	if err := s.Apply("roster.changed", json.RawMessage(`{"bots":[{"id":"b","name":"Bot"}],"chats":[{"id":"c","bot_ids":["b"],"title":"Named"}]}`)); err != nil {
		t.Fatal(err)
	}
	chat := s.Chat("c")
	if chat.Title != "Named" || chat.UnreadCount != 4 {
		t.Fatalf("roster dropped unread: %+v", chat)
	}
}

func TestNativeStoreMarkReadSendsCLIAndSurvivesSnapshot(t *testing.T) {
	s := NewNativeStore()
	var calls []struct {
		method string
		params map[string]any
	}
	s.Perform = func(method string, params map[string]any) {
		calls = append(calls, struct {
			method string
			params map[string]any
		}{method, params})
	}
	if err := s.Apply("snapshot", json.RawMessage(`{
		"has_identity": true,
		"identity_id": "account",
		"bots": [{"id": "b", "name": "Bot"}],
		"chats": [
			{"id": "c1", "bot_ids": ["b"], "messages": []},
			{"id": "c2", "bot_ids": ["b"], "unread_count": 3, "messages": []}
		]
	}`)); err != nil {
		t.Fatal(err)
	}
	// Snapshot focuses c1 and may mark it read; c2 stays unread until selected.
	if s.Chat("c2").UnreadCount != 3 {
		t.Fatalf("c2 unread=%d", s.Chat("c2").UnreadCount)
	}
	calls = nil
	s.SelectChat("c2")
	if s.Chat("c2").UnreadCount != 0 {
		t.Fatal("SelectChat left local unread")
	}
	if len(calls) != 1 || calls[0].method != "chats.mark_read" || calls[0].params["chat_id"] != "c2" {
		t.Fatalf("mark_read not sent: %+v", calls)
	}
	// Next snapshot after the CLI accepted mark_read reports unread_count: 0 — count stays cleared.
	if err := s.Apply("snapshot", json.RawMessage(`{
		"has_identity": true,
		"identity_id": "account",
		"bots": [{"id": "b", "name": "Bot"}],
		"chats": [
			{"id": "c1", "bot_ids": ["b"], "messages": []},
			{"id": "c2", "bot_ids": ["b"], "unread_count": 0, "messages": []}
		]
	}`)); err != nil {
		t.Fatal(err)
	}
	if s.Selected != "c2" || s.Chat("c2").UnreadCount != 0 {
		t.Fatalf("snapshot reverted unread: selected=%q unread=%d", s.Selected, s.Chat("c2").UnreadCount)
	}
}

func TestNativeStoreOwnMessageDoesNotBumpUnread(t *testing.T) {
	s := NewNativeStore()
	if err := s.Apply("snapshot", json.RawMessage(`{
		"has_identity": true,
		"identity_id": "account",
		"bots": [{"id": "b", "name": "Bot"}],
		"chats": [
			{"id": "focused", "bot_ids": ["b"], "messages": []},
			{"id": "other", "bot_ids": ["b"], "unread_count": 1, "messages": []}
		]
	}`)); err != nil {
		t.Fatal(err)
	}
	s.Selected = "focused"
	before := s.Chat("other").UnreadCount
	if err := s.Apply("message.added", json.RawMessage(`{
		"chat_id": "other",
		"message": {
			"id": "from-phone",
			"chat_id": "other",
			"author": {"kind": "you"},
			"body": {"kind": "text", "text": "sent elsewhere"},
			"state": {"kind": "complete"},
			"created_at": 1
		}
	}`)); err != nil {
		t.Fatal(err)
	}
	other := s.Chat("other")
	if len(other.Messages) != 1 || other.Messages[0].ID != "from-phone" {
		t.Fatalf("own message not stored: %+v", other.Messages)
	}
	if other.UnreadCount != before {
		t.Fatalf("own message bumped unread from %d to %d", before, other.UnreadCount)
	}
}

func TestNativeStoreMessageUpdatedUpsertsUnknownID(t *testing.T) {
	s := NewNativeStore()
	if err := s.Apply("snapshot", json.RawMessage(`{
		"has_identity": true,
		"identity_id": "account",
		"bots": [{"id": "b", "name": "Bot"}],
		"chats": [{"id": "c", "bot_ids": ["b"], "messages": []}]
	}`)); err != nil {
		t.Fatal(err)
	}
	if err := s.Apply("message.updated", json.RawMessage(`{
		"chat_id": "c",
		"message": {
			"id": "missed-added",
			"chat_id": "c",
			"author": {"kind": "bot", "bot_id": "b"},
			"body": {"kind": "text", "text": "recovered"},
			"state": {"kind": "complete"},
			"created_at": 1
		}
	}`)); err != nil {
		t.Fatal(err)
	}
	chat := s.Chat("c")
	if len(chat.Messages) != 1 || chat.Messages[0].ID != "missed-added" || chat.Messages[0].Body.Text != "recovered" {
		t.Fatalf("updated did not upsert: %+v", chat.Messages)
	}
	if chat.UnreadCount != 0 {
		t.Fatalf("upsert bumped unread: %d", chat.UnreadCount)
	}
}

func TestFormatStampAndMarkdownPlain(t *testing.T) {
	if PlainText(MarkdownBlocks("**bold** and `code`")) != "bold and code" {
		t.Fatal(PlainText(MarkdownBlocks("**bold** and `code`")))
	}
	now := Now()
	if !IsToday(now) {
		t.Fatal("IsToday rejected now")
	}
	if Stamp(now) == "" || Clock(now) == "" {
		t.Fatal("formatters empty")
	}
}
