package main

import (
	"encoding/json"
	"errors"
	"reflect"
	"testing"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestNativeLookCombinedSaveRetainsFailedDraft(t *testing.T) {
	var complete func(json.RawMessage, error)
	var payload map[string]json.RawMessage
	calls := 0
	e := newNativeLookEditor(model.NativeBot{ID: "bot"}, func(method string, raw json.RawMessage, done func(json.RawMessage, error)) {
		if method != "bots.update" {
			t.Fatal(method)
		}
		calls++
		_ = json.Unmarshal(raw, &payload)
		complete = done
	})
	e.selection = "working"
	e.edit("expression", "happy")
	e.photo = &FileInfo{Path: "/prepared.png", Name: "photo.png", Mime: "image/png"}
	e.photoID = "photo-id"
	draft := string(e.look)
	e.save()
	e.save()
	if calls != 1 || e.CanDismiss() || e.Done() {
		t.Fatal("pending save allowed duplicate dispatch or dismissal")
	}
	if string(payload["id"]) != `"bot"` || string(payload["look"]) != draft {
		t.Fatal("combined save lost look or bot identity")
	}
	var photo map[string]string
	_ = json.Unmarshal(payload["avatar"], &photo)
	if !reflect.DeepEqual(photo, map[string]string{"id": "photo-id", "path": "/prepared.png", "name": "photo.png", "mime": "image/png"}) {
		t.Fatal(photo)
	}
	complete(nil, errors.New("offline"))
	if !e.CanDismiss() || e.Done() || !e.HasIntent() || string(e.look) != draft || e.photo == nil || e.errorText != "offline" {
		t.Fatal("failure lost draft or error")
	}
	e.save()
	if calls != 2 {
		t.Fatal("failed save cannot retry")
	}
	complete(nil, nil)
	if !e.Done() || !e.CanDismiss() || e.HasIntent() {
		t.Fatal("successful awaited save did not release intent")
	}
}

func TestNativeLookPhotoEditOmitsUnchangedFutureLook(t *testing.T) {
	future := json.RawMessage(`{"version":2,"base":{"future":true}}`)
	var payload map[string]json.RawMessage
	e := newNativeLookEditor(model.NativeBot{ID: "bot", Look: future}, func(_ string, raw json.RawMessage, done func(json.RawMessage, error)) {
		_ = json.Unmarshal(raw, &payload)
		done(nil, nil)
	})
	e.removePhoto = true
	e.save()
	if _, ok := payload["look"]; ok {
		t.Fatal("photo-only edit authored unsupported saved look")
	}
	if string(payload["avatar"]) != "null" || !e.Done() {
		t.Fatal("photo removal was not saved independently")
	}
}

func TestNativeLookSparseExplicitStateAndInvalidHue(t *testing.T) {
	calls := 0
	e := newNativeLookEditor(model.NativeBot{ID: "bot", Look: json.RawMessage(`{"version":1,"base":{"expression":"happy"}}`)}, func(_ string, _ json.RawMessage, _ func(json.RawMessage, error)) { calls++ })
	e.selection = "working"
	e.edit("expression", "happy")
	var look struct {
		States map[string]map[string]any `json:"states"`
	}
	_ = json.Unmarshal(e.look, &look)
	if look.States["working"]["expression"] != "happy" {
		t.Fatal("explicit equal-to-base override disappeared")
	}
	e.fields["working"] = map[string]string{"hue": "-"}
	e.edit("hue", "-")
	e.save()
	if calls != 0 || !e.HasIntent() || e.errorText == "" {
		t.Fatal("invalid partial hue was discarded or dispatched")
	}
	e.reset()
	e.selection = "base"
	e.reset()
	if string(e.look) != "null" || e.photo != nil || e.removePhoto {
		t.Fatal("reset changed independent photo intent")
	}
}

func TestNativeLookSurfaceBlocksPendingCancelAndRetainsFailure(t *testing.T) {
	var complete func(json.RawMessage, error)
	e := newNativeLookEditor(model.NativeBot{ID: "bot", Name: "Bot"}, func(_ string, _ json.RawMessage, done func(json.RawMessage, error)) { complete = done })
	e.edit("shape", "cloud")
	tt := ui.NewTester(func(c *ui.Context) { ui.Column(c).Fill().Children(func() { e.View(c) }) }, 600, 900)
	if err := tt.Click("Save"); err != nil {
		t.Fatal(err)
	}
	if !tt.HasText("Saving…") || e.CanDismiss() {
		t.Fatal("surface did not enter pending state")
	}
	if err := tt.Click("Cancel"); err == nil {
		t.Fatal("pending surface retained cancel action")
	}
	complete(nil, errors.New("save failed"))
	tt.Frame()
	if !tt.HasText("save failed") || !e.HasIntent() {
		t.Fatal("surface lost failed draft/error")
	}
	if err := tt.Click("Cancel"); err != nil {
		t.Fatal(err)
	}
	if !e.Done() {
		t.Fatal("explicit cancel did not close draft")
	}
}
