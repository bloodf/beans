package main

import "math"

type nativeGroupAvatarSlot struct {
	BotID      string
	X, Y, Size float32
}

// Slots preserve roster order and the shared AvatarCluster geometry.
func nativeGroupAvatarSlots(botIDs []string, slot float32) []nativeGroupAvatarSlot {
	count := min(len(botIDs), 4)
	if count == 0 {
		return nil
	}
	slots := make([]nativeGroupAvatarSlot, count)
	if count == 1 {
		slots[0] = nativeGroupAvatarSlot{BotID: botIDs[0], X: 1, Y: 1, Size: slot - 2}
		return slots
	}
	size := float32(math.Floor(float64(slot)*0.6 + 0.5))
	free := slot - size
	for i := range slots {
		slots[i] = nativeGroupAvatarSlot{BotID: botIDs[i], Size: size}
	}
	switch count {
	case 2:
		slots[1].X, slots[1].Y = free, free
	case 3:
		slots[1].X = free
		slots[2].X, slots[2].Y = free/2, free
	case 4:
		slots[1].X = free
		slots[2].Y = free
		slots[3].X, slots[3].Y = free, free
	}
	return slots
}
