package model

import (
	"encoding/json"
	"errors"
	"slices"
	"strings"
)

// NativeStore consumes only Session-admitted events. All access belongs to the
// host's ordered main-thread queue, including asynchronous completions.
type NativeStore struct {
	Epoch       uint64
	Connected   bool
	HasIdentity bool
	Selected    string
	Bots        []NativeBot
	Chats       []NativeChat
	Drafts      map[string]*NativeDraft
	Error       string
}

type NativeBot struct {
	ID       string          `json:"id"`
	Name     string          `json:"name"`
	RunnerID string          `json:"runner_id"`
	Provider string          `json:"provider"`
	Model    *string         `json:"model"`
	Avatar   json.RawMessage `json:"avatar"`
	// Keep future appearance keys intact; unrelated edits never author look.
	Look json.RawMessage `json:"look"`
}
type NativeMessage struct {
	ID     string `json:"id"`
	ChatID string `json:"chat_id"`
	Author struct {
		Kind  string `json:"kind"`
		BotID string `json:"bot_id"`
	} `json:"author"`
	Body struct {
		Kind        string            `json:"kind"`
		Text        string            `json:"text"`
		Summary     string            `json:"summary"`
		Detail      string            `json:"detail"`
		Attachments []json.RawMessage `json:"attachments"`
		ReplyTo     json.RawMessage   `json:"reply_to"`
	} `json:"body"`
	State struct {
		Kind  string `json:"kind"`
		Error string `json:"error"`
	} `json:"state"`
}
type NativeChat struct {
	ID       string          `json:"id"`
	Title    string          `json:"title"`
	BotIDs   []string        `json:"bot_ids"`
	Messages []NativeMessage `json:"messages"`
	HasMore  bool            `json:"has_more"`
}
type NativeAttachment struct {
	ID     string `json:"id"`
	Path   string `json:"path"`
	Name   string `json:"name"`
	Mime   string `json:"mime"`
	Size   int64  `json:"size"`
	Width  *int   `json:"width,omitempty"`
	Height *int   `json:"height,omitempty"`
}
type NativeDraft struct {
	Attachments []NativeAttachment
	ReplyTo     string
	Mentions    []string
	Text        string
	Sending     bool
	Revision    uint64
	Error       string
}

func NewNativeStore() *NativeStore { return &NativeStore{Drafts: map[string]*NativeDraft{}} }
func (s *NativeStore) Fence() {
	s.Epoch++
	for _, d := range s.Drafts {
		d.Sending = false
	}
	s.Error = ""
}
func (s *NativeStore) Reset() {
	s.Fence()
	s.HasIdentity = false
	s.Bots = nil
	s.Chats = nil
	s.Selected = ""
	s.Drafts = map[string]*NativeDraft{}
}
func (s *NativeStore) Chat(id string) *NativeChat {
	for i := range s.Chats {
		if s.Chats[i].ID == id {
			return &s.Chats[i]
		}
	}
	return nil
}
func (s *NativeStore) Draft(id string) *NativeDraft {
	d := s.Drafts[id]
	if d == nil {
		d = &NativeDraft{}
		s.Drafts[id] = d
	}
	return d
}
func (s *NativeStore) Title(chat *NativeChat) string {
	if chat.Title != "" {
		return chat.Title
	}
	names := make([]string, 0, len(chat.BotIDs))
	for _, id := range chat.BotIDs {
		for _, b := range s.Bots {
			if b.ID == id {
				names = append(names, b.Name)
				break
			}
		}
	}
	return strings.Join(names, ", ")
}
func (s *NativeStore) Apply(name string, data json.RawMessage) error {
	if !s.HasIdentity && name != "snapshot" && name != "identity.changed" {
		return nil
	}
	switch name {
	case "snapshot", "identity.changed":
		has, err := identityOf(data)
		if err != nil {
			return err
		}
		if !has {
			s.Reset()
			return nil
		}
		s.HasIdentity = true
		if name == "identity.changed" {
			return nil
		}
		fallthrough
	case "roster.changed":
		var v struct {
			Bots  []NativeBot  `json:"bots"`
			Chats []NativeChat `json:"chats"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		for i := range v.Chats {
			if old := s.Chat(v.Chats[i].ID); old != nil {
				if name == "roster.changed" {
					v.Chats[i].Messages = old.Messages
					v.Chats[i].HasMore = old.HasMore
				} else {
					// Preserve only history preceding the authoritative newest page.
					if len(v.Chats[i].Messages) > 0 {
						first := v.Chats[i].Messages[0].ID
						for j, m := range old.Messages {
							if m.ID == first {
								v.Chats[i].Messages = append(slices.Clone(old.Messages[:j]), v.Chats[i].Messages...)
								break
							}
						}
					}
				}
			}
		}
		s.Bots, s.Chats = v.Bots, v.Chats
		for id := range s.Drafts {
			if s.Chat(id) == nil {
				delete(s.Drafts, id)
			}
		}
		if s.Chat(s.Selected) == nil {
			s.Selected = ""
			if len(s.Chats) > 0 {
				s.Selected = s.Chats[0].ID
			}
		}
	case "message.added", "message.updated":
		var v struct {
			ChatID  string        `json:"chat_id"`
			Message NativeMessage `json:"message"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		if c := s.Chat(v.ChatID); c != nil {
			for i := range c.Messages {
				if c.Messages[i].ID == v.Message.ID {
					c.Messages[i] = v.Message
					return nil
				}
			}
			c.Messages = append(c.Messages, v.Message)
		}
	case "message.removed":
		var v struct {
			ChatID string `json:"chat_id"`
			ID     string `json:"message_id"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		if c := s.Chat(v.ChatID); c != nil {
			c.Messages = slices.DeleteFunc(c.Messages, func(m NativeMessage) bool { return m.ID == v.ID })
		}
	case "chat.removed":
		var v struct {
			ID string `json:"chat_id"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		s.Chats = slices.DeleteFunc(s.Chats, func(c NativeChat) bool { return c.ID == v.ID })
		delete(s.Drafts, v.ID)
		if s.Selected == v.ID {
			s.Selected = ""
		}
	}
	return nil
}

// BeginSend captures immutable intent. Completion cannot erase edits made while
// the CLI works, or affect another account/connection after teardown.
func (s *NativeStore) BeginSend(id, messageID string) (map[string]any, func(error), error) {
	if !s.Connected || !s.HasIdentity || s.Chat(id) == nil {
		return nil, nil, errors.New("The Beans CLI is not running")
	}
	d := s.Draft(id)
	text := strings.TrimSpace(d.Text)
	if d.Sending || (text == "" && len(d.Attachments) == 0) {
		return nil, nil, errors.New("Write a message before sending")
	}
	epoch, revision, original := s.Epoch, d.Revision, d.Text
	d.Sending = true
	d.Error = ""
	params := map[string]any{"chat_id": id, "message_id": messageID, "text": text, "mentions": slices.Clone(d.Mentions), "attachments": slices.Clone(d.Attachments)}
	if d.ReplyTo != "" {
		params["reply_to"] = d.ReplyTo
	}
	return params, func(err error) {
		if s.Epoch != epoch || s.Drafts[id] != d {
			return
		}
		d.Sending = false
		if err != nil {
			d.Error = err.Error()
			return
		}
		if d.Revision == revision && d.Text == original {
			d.Text = ""
			d.Attachments = nil
			d.Mentions = nil
			d.ReplyTo = ""
			d.Revision++
		}
	}, nil
}
