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
	Epoch          uint64
	Connected      bool
	HasIdentity    bool
	Selected       string
	Bots           []NativeBot
	Chats          []NativeChat
	Drafts         map[string]*NativeDraft
	Error          string
	Running        map[string]string
	LoadingOlder   map[string]bool
	AccountID      string
	ArchivedDrafts map[string]map[string]*NativeDraft
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

// NativeReplyQuote is the quoted message a reply points at, as the CLI sends it.
type NativeReplyQuote struct {
	MessageID string `json:"message_id"`
	Author    struct {
		Kind  string `json:"kind"`
		BotID string `json:"bot_id"`
	} `json:"author"`
	Text string `json:"text"`
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
		ReplyTo     *NativeReplyQuote `json:"reply_to"`
	} `json:"body"`
	State struct {
		Kind  string `json:"kind"`
		Error string `json:"error"`
	} `json:"state"`
	CreatedAt float64 `json:"created_at"`
}
type NativeChat struct {
	ID          string          `json:"id"`
	Title       string          `json:"title"`
	BotIDs      []string        `json:"bot_ids"`
	Messages    []NativeMessage `json:"messages"`
	HasMore     bool            `json:"has_more"`
	UnreadCount int             `json:"unread_count"`
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

func NewNativeStore() *NativeStore {
	return &NativeStore{Drafts: map[string]*NativeDraft{}, Running: map[string]string{}, LoadingOlder: map[string]bool{}}
}
func (s *NativeStore) Fence() {
	s.Epoch++
	s.Connected = false
	for _, d := range s.Drafts {
		d.Sending = false
	}
	s.Error = ""
	s.Running = map[string]string{}
	s.LoadingOlder = map[string]bool{}
}
func (s *NativeStore) Reset() {
	s.Fence()
	s.HasIdentity = false
	s.Bots = nil
	s.Chats = nil
	s.Selected = ""
	if s.HasDrafts() && s.AccountID != "" {
		if s.ArchivedDrafts == nil {
			s.ArchivedDrafts = map[string]map[string]*NativeDraft{}
		}
		s.ArchivedDrafts[s.AccountID] = s.Drafts
	}
	s.Drafts = map[string]*NativeDraft{}
	s.AccountID = ""
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

// DisplayText is the message body for the native transcript: Markdown folded to plain words when
// the body is text, otherwise the summary the CLI already prepared.
func (m NativeMessage) DisplayText() string {
	text := m.Body.Text
	if text == "" {
		text = m.Body.Summary
	}
	if m.Body.Kind == "text" && text != "" {
		return PlainText(MarkdownBlocks(text))
	}
	return text
}

// SelectChat selects a chat and clears its local unread count, matching the Solid store's focus
// path. The host still owns `chats.mark_read` over the CLI.
func (s *NativeStore) SelectChat(id string) {
	if s.Chat(id) == nil {
		return
	}
	s.Selected = id
	s.MarkRead(id)
}

// MarkRead clears local unread for a chat without touching other chats' counts.
func (s *NativeStore) MarkRead(id string) {
	chat := s.Chat(id)
	if chat == nil || chat.UnreadCount <= 0 {
		return
	}
	chat.UnreadCount = 0
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
			Bots         []NativeBot  `json:"bots"`
			Chats        []NativeChat `json:"chats"`
			RunningTurns []struct {
				ID     string `json:"job_id"`
				ChatID string `json:"chat_id"`
			} `json:"running_turns"`
			IdentityID string `json:"identity_id"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			// Malformed roster/snapshot must not wipe an admitted account.
			return err
		}
		if name == "snapshot" {
			if v.IdentityID == "" {
				s.Connected = false
				return errors.New("CLI account identity evidence is missing")
			}
			if s.AccountID != "" && s.AccountID != v.IdentityID {
				s.Reset()
			}
			s.AccountID = v.IdentityID
			if drafts := s.ArchivedDrafts[v.IdentityID]; drafts != nil {
				s.Drafts = drafts
				delete(s.ArchivedDrafts, v.IdentityID)
			}
			s.HasIdentity = true
			s.Connected = true
			s.Running = map[string]string{}
			for _, turn := range v.RunningTurns {
				s.Running[turn.ID] = turn.ChatID
			}
		}
		for i := range v.Chats {
			if old := s.Chat(v.Chats[i].ID); old != nil {
				if name == "roster.changed" {
					v.Chats[i].Messages = old.Messages
					v.Chats[i].HasMore = old.HasMore
					// Roster updates omit unread; keep the live count unless the wire sent one.
					if !chatUnreadPresent(data, v.Chats[i].ID) {
						v.Chats[i].UnreadCount = old.UnreadCount
					}
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
		previous := s.Selected
		if s.Chat(s.Selected) == nil {
			s.Selected = ""
			if len(s.Chats) > 0 {
				s.Selected = s.Chats[0].ID
			}
		}
		// Clear unread only when a snapshot (or lost selection) establishes focus — not on every roster.
		if name == "snapshot" || (previous != "" && previous != s.Selected) {
			if s.Selected != "" {
				s.MarkRead(s.Selected)
			}
		}
	case "message.added", "message.updated":
		var v struct {
			ChatID  string          `json:"chat_id"`
			Message json.RawMessage `json:"message"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		msg, err := decodeNativeMessage(v.Message)
		if err != nil {
			return err
		}
		c := s.Chat(v.ChatID)
		if c == nil {
			return nil
		}
		for i := range c.Messages {
			if c.Messages[i].ID == msg.ID {
				c.Messages[i] = msg
				return nil
			}
		}
		if name == "message.added" {
			c.Messages = append(c.Messages, msg)
			if s.Selected != v.ChatID {
				c.UnreadCount++
			}
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
	case "job.started", "job.finished":
		var v struct {
			ID     string `json:"job_id"`
			ChatID string `json:"chat_id"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		if name == "job.started" {
			s.Running[v.ID] = v.ChatID
		} else {
			delete(s.Running, v.ID)
		}
	case "chat.removed":
		var v struct {
			ID string `json:"chat_id"`
		}
		if err := json.Unmarshal(data, &v); err != nil {
			return err
		}
		s.Chats = slices.DeleteFunc(s.Chats, func(c NativeChat) bool { return c.ID == v.ID })
		if s.Selected == v.ID {
			s.Selected = ""
		}
	default:
		// Unsupported Beans/upstream events stay ignored; stored state is unchanged.
	}
	return nil
}

func chatUnreadPresent(data json.RawMessage, chatID string) bool {
	var probe struct {
		Chats []struct {
			ID          string `json:"id"`
			UnreadCount *int   `json:"unread_count"`
		} `json:"chats"`
	}
	if err := json.Unmarshal(data, &probe); err != nil {
		return false
	}
	for _, chat := range probe.Chats {
		if chat.ID == chatID {
			return chat.UnreadCount != nil
		}
	}
	return false
}

func decodeNativeMessage(data json.RawMessage) (NativeMessage, error) {
	var msg NativeMessage
	if err := json.Unmarshal(data, &msg); err != nil {
		return NativeMessage{}, err
	}
	if msg.ID == "" {
		return NativeMessage{}, errors.New("message id is missing")
	}
	// Cross-check Beans wire decoding so unsupported body shapes cannot invent a corrupt quote.
	var wire WireMessage
	if err := json.Unmarshal(data, &wire); err != nil {
		return NativeMessage{}, err
	}
	decoded := ToMessage(wire)
	if decoded != nil && decoded.ReplyTo != nil {
		quote := NativeReplyQuote{MessageID: decoded.ReplyTo.MessageID, Text: decoded.ReplyTo.Text}
		switch decoded.ReplyTo.Author.Kind {
		case AuthorYou:
			quote.Author.Kind = "you"
		case AuthorBot:
			quote.Author.Kind = "bot"
			quote.Author.BotID = decoded.ReplyTo.Author.BotID
		default:
			quote.Author.Kind = "system"
		}
		msg.Body.ReplyTo = &quote
	}
	return msg, nil
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

// BeginOlder admits one page against its original first row and account epoch.
func (s *NativeStore) BeginOlder(id string) (map[string]string, func(json.RawMessage, error), error) {
	c := s.Chat(id)
	if !s.Connected || c == nil || !c.HasMore || len(c.Messages) == 0 || s.LoadingOlder[id] {
		return nil, nil, errors.New("Older messages are not available")
	}
	epoch, first := s.Epoch, c.Messages[0].ID
	s.LoadingOlder[id] = true
	return map[string]string{"chat_id": id, "before": first}, func(data json.RawMessage, err error) {
		if epoch != s.Epoch {
			return
		}
		delete(s.LoadingOlder, id)
		current := s.Chat(id)
		if current == nil || len(current.Messages) == 0 || current.Messages[0].ID != first {
			return
		}
		if err != nil {
			s.Error = err.Error()
			return
		}
		var page struct {
			Messages []NativeMessage `json:"messages"`
			HasMore  bool            `json:"has_more"`
		}
		if err := json.Unmarshal(data, &page); err != nil {
			s.Error = err.Error()
			return
		}
		known := make(map[string]bool, len(current.Messages))
		for _, m := range current.Messages {
			known[m.ID] = true
		}
		older := make([]NativeMessage, 0, len(page.Messages))
		for _, m := range page.Messages {
			if !known[m.ID] {
				older = append(older, m)
				known[m.ID] = true
			}
		}
		current.Messages = append(older, current.Messages...)
		current.HasMore = page.HasMore
	}, nil
}

func (s *NativeStore) HasDrafts() bool {
	for _, d := range s.Drafts {
		if d.Text != "" || len(d.Attachments) != 0 || d.ReplyTo != "" || len(d.Mentions) != 0 || d.Sending {
			return true
		}
	}
	return false
}
func (s *NativeStore) HasOwnedIntent() bool { return s.HasDrafts() || len(s.ArchivedDrafts) != 0 }
