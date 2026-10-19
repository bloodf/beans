package main

import (
	"encoding/json"
	"strings"
	"testing"

	"github.com/bloodf/beans/desktop/model"
)

func TestNativeSharedGeometryUsesSavedLook(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	a := newNativeAvatars(r)
	bot := model.NativeBot{ID: "bot-native", Look: json.RawMessage(`{"version":1,"base":{"palette":{"head":"#ABCDEF"}},"states":{"working":{"palette":{"eye":"#123456"}}}}`)}
	f, err := a.frame(bot, "working")
	if err != nil {
		t.Fatal(err)
	}
	if f.Head != "#ABCDEF" || f.Eye != "#123456" {
		t.Fatalf("lost shared palette inheritance: %+v", f)
	}
	if len(f.Core.Segments) != 24 || len(f.Extra.Segments) != 4 {
		t.Fatal("shared geometry contour contract changed")
	}
}
func TestNativeSharedMemoryConsentAndMasking(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := &nativeMemory{runtime: r}
	id, err := m.newDraft("b", nil)
	if err != nil {
		t.Fatal(err)
	}
	m.draft(id, "connection", "a")
	m.draft(id, "captureConversation", true)
	if _, err = m.draft(id, "request", nil); err == nil || !strings.Contains(err.Error(), "plaintext_consent_required") {
		t.Fatalf("consent: %v", err)
	}
	m.draft(id, "approvePlaintext", nil)
	if _, err = m.draft(id, "request", nil); err != nil {
		t.Fatal(err)
	}
	m.draft(id, "connection", "b")
	m.draft(id, "connection", "a")
	if _, err = m.draft(id, "request", nil); err == nil {
		t.Fatal("connection change retained consent")
	}
	masked, err := m.masked("connections", json.RawMessage(`{"schema_version":1,"connections":[{"id":"c","revision":{"counter":1,"device_id":"d"},"backend":"hindsight","name":"Memory","has_secret":true,"embedding_profile":null,"availability":"supported","reason":null,"secret":"private","endpoint":"private"}],"embeddings":[],"bots":[]}`))
	if err != nil {
		t.Fatal(err)
	}
	if strings.Contains(string(masked), "private") {
		t.Fatal("private fields escaped masked boundary")
	}
}
