package model

import (
	"encoding/json"
	"errors"
)

// Session orders bootstrap and live events on the host's main-thread queue.
// Connection and account teardown invalidate in-flight bootstrap completions.
// The CLI remains authoritative for identity and account data.
type Session struct {
	generation    uint64
	connected     bool
	bootstrapping bool
	pending       []sessionEvent
	identity      func(bool)
	publish       func(string, json.RawMessage)
}

type sessionEvent struct {
	name string
	data json.RawMessage
}

func NewSession(identity func(bool), publish func(string, json.RawMessage)) *Session {
	return &Session{identity: identity, publish: publish}
}

func (s *Session) Connect() uint64 {
	s.generation++
	s.connected = true
	s.bootstrapping = true
	s.pending = nil
	return s.generation
}

func (s *Session) Disconnect() {
	s.generation++
	s.connected = false
	s.bootstrapping = false
	s.pending = nil
}

func identityOf(data json.RawMessage) (bool, error) {
	var value struct {
		HasIdentity *bool `json:"has_identity"`
	}
	if err := json.Unmarshal(data, &value); err != nil {
		return false, err
	}
	if value.HasIdentity == nil {
		return false, errors.New("CLI identity evidence is missing")
	}
	return *value.HasIdentity, nil
}

func (s *Session) Bootstrap(generation uint64, data json.RawMessage) error {
	if !s.connected || !s.bootstrapping || generation != s.generation {
		return nil
	}
	has, err := identityOf(data)
	if err != nil {
		return err
	}
	s.identity(has)
	s.publish("snapshot", data)
	pending := s.pending
	s.pending = nil
	s.bootstrapping = false
	for _, event := range pending {
		s.Event(generation, event.name, event.data)
	}
	return nil
}

func (s *Session) Event(generation uint64, name string, data json.RawMessage) {
	if !s.connected || generation != s.generation {
		return
	}
	if name == "identity.changed" || name == "snapshot" {
		has, err := identityOf(data)
		if err != nil {
			return
		}
		if !has {
			s.pending = nil
			s.bootstrapping = false
			s.identity(false)
			s.publish(name, data)
			return
		}
		if !s.bootstrapping {
			s.identity(true)
		}
	}
	if s.bootstrapping {
		s.pending = append(s.pending, sessionEvent{name, data})
		return
	}
	s.publish(name, data)
}
