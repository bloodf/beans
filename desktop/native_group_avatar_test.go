package main

import (
	"reflect"
	"testing"
)

func TestNativeGroupAvatarSlotsConsumer(t *testing.T) {
	ids := []string{"back", "right", "left", "front", "excluded"}
	for _, tc := range []struct {
		count int
		want  []nativeGroupAvatarSlot
	}{
		{0, nil},
		{1, []nativeGroupAvatarSlot{{"back", 1, 1, 36}}},
		{2, []nativeGroupAvatarSlot{{"back", 0, 0, 23}, {"right", 15, 15, 23}}},
		{3, []nativeGroupAvatarSlot{{"back", 0, 0, 23}, {"right", 15, 0, 23}, {"left", 7.5, 15, 23}}},
		{4, []nativeGroupAvatarSlot{{"back", 0, 0, 23}, {"right", 15, 0, 23}, {"left", 0, 15, 23}, {"front", 15, 15, 23}}},
		{5, []nativeGroupAvatarSlot{{"back", 0, 0, 23}, {"right", 15, 0, 23}, {"left", 0, 15, 23}, {"front", 15, 15, 23}}},
	} {
		if got := nativeGroupAvatarSlots(ids[:tc.count], 38); !reflect.DeepEqual(got, tc.want) {
			t.Fatalf("%d members: got %+v, want %+v", tc.count, got, tc.want)
		}
	}
	// Smaller palette slots use the same rounding, not the sidebar's size.
	want := []nativeGroupAvatarSlot{{"back", 0, 0, 13}, {"right", 9, 9, 13}}
	if got := nativeGroupAvatarSlots(ids[:2], 22); !reflect.DeepEqual(got, want) {
		t.Fatalf("palette slots: got %+v, want %+v", got, want)
	}
	if !reflect.DeepEqual(ids, []string{"back", "right", "left", "front", "excluded"}) {
		t.Fatal("slot selection mutated roster order")
	}
}
