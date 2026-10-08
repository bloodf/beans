package main

import (
	"encoding/json"
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
	"testing"
)

func TestNativeSurfaceSmoke(t *testing.T) {
	n := &nativeDesktop{store: model.NewNativeStore()}
	n.store.Connected = true
	if err := n.store.Apply("snapshot", json.RawMessage(`{"has_identity":true,"bots":[{"id":"b","name":"Fixture Bot"}],"chats":[{"id":"c","bot_ids":["b"],"messages":[{"id":"m","author":{"kind":"bot","bot_id":"b"},"body":{"kind":"text","text":"Fixture transcript"},"state":{"kind":"complete"}}]},{"id":"d","title":"Second chat","messages":[]}]}`)); err != nil {
		t.Fatal(err)
	}
	tt := ui.NewTester(n.view, 1100, 760)
	if !tt.HasText("Fixture transcript") {
		t.Fatal(tt.Texts())
	}
	if err := tt.Click("Second chat"); err != nil {
		t.Fatal(err)
	}
	if n.store.Selected != "d" || tt.HasText("Fixture transcript") {
		t.Fatal("selection failed")
	}
	if err := tt.Click("Message"); err != nil {
		t.Fatal(err)
	}
	tt.Type("long draft\nsecond line\nthird line")
	if n.store.Draft("d").Text != "long draft\nsecond line\nthird line" {
		t.Fatal("multiline input lost")
	}
	if _, ok := tt.TextCaret(); !ok {
		t.Fatal("composer caret unavailable")
	}
	t.Log("native surface: admitted transcript, sidebar selection, multiline draft rendered")
}
