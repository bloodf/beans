package main

import (
	"encoding/json"
	"reflect"
	"testing"
)

func TestNativeMotionRetargetsDisplayedGeometry(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := newNativeAvatarMotion(r)
	if err = m.SetTarget("bot", nil, "idle"); err != nil {
		t.Fatal(err)
	}
	if _, moving, err := m.Frame(0, true, false); err != nil || !moving {
		t.Fatalf("idle clock: %v %v", moving, err)
	}
	if err = m.SetTarget("bot", nil, "working"); err != nil {
		t.Fatal(err)
	}
	_, _, _ = m.Frame(10, true, false)
	_, _, _ = m.Frame(150, true, false)
	displayed := append(json.RawMessage(nil), m.current...)
	if err = m.SetTarget("bot", nil, "error"); err != nil {
		t.Fatal(err)
	}
	_, _, err = m.Frame(150, true, false)
	if err != nil {
		t.Fatal(err)
	}
	var a, b any
	_ = json.Unmarshal(displayed, &a)
	_ = json.Unmarshal(m.current, &b)
	if !reflect.DeepEqual(a, b) {
		t.Fatal("interrupted morph jumped away from displayed geometry")
	}
}

func TestNativeMotionSuppressionAndReset(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	m := newNativeAvatarMotion(r)
	if err = m.SetTarget("bot", nil, "working"); err != nil {
		t.Fatal(err)
	}
	for _, gate := range []struct{ visible, reduced bool }{{false, false}, {true, true}} {
		frame, moving, err := m.Frame(1000, gate.visible, gate.reduced)
		if err != nil || moving {
			t.Fatalf("suppression: %v %v", moving, err)
		}
		var want avatarFrame
		if err = r.eval(`return avatarFrame(input,0,0);`, m.target, &want); err != nil {
			t.Fatal(err)
		}
		if !reflect.DeepEqual(frame, want) {
			t.Fatal("suppression did not draw static endpoint")
		}
	}
	if err = m.SetTarget("bot", json.RawMessage(`{"version":1,"base":{"motion":false}}`), "idle"); err != nil {
		t.Fatal(err)
	}
	if _, moving, err := m.Frame(2000, true, false); err != nil || moving {
		t.Fatalf("saved motion gate: %v %v", moving, err)
	}
	m.Reset()
	if _, moving, err := m.Frame(3000, true, false); err == nil || moving {
		t.Fatal("reset retained previous account geometry")
	}
}
