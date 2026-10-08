package main

import (
	"encoding/binary"
	"testing"
)

type controlTestACE struct {
	kind, flags byte
	mask        uint32
	sid         []byte
}

func controlTestACL(entries ...controlTestACE) []byte {
	data := []byte{2, 0, 0, 0, byte(len(entries)), 0, 0, 0}
	for _, entry := range entries {
		data = append(data, entry.kind, entry.flags)
		data = binary.LittleEndian.AppendUint16(data, uint16(8+len(entry.sid)))
		data = binary.LittleEndian.AppendUint32(data, entry.mask)
		data = append(data, entry.sid...)
	}
	binary.LittleEndian.PutUint16(data[2:4], uint16(len(data)))
	return data
}

func TestWindowsControlPrivacyRejectsForeignOwnerAndEffectiveGrants(t *testing.T) {
	user := []byte{1, 1, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0}
	other := []byte{1, 1, 0, 0, 0, 0, 0, 5, 22, 0, 0, 0}
	admins := []byte{1, 2, 0, 0, 0, 0, 0, 5, 32, 0, 0, 0, 32, 2, 0, 0}
	own := controlTestACE{mask: 0x1F01FF, sid: user}
	if !desktopControlACLPrivate(user, user, controlTestACL(own), true) {
		t.Fatal("private user ACL rejected")
	}
	if desktopControlACLPrivate(other, user, controlTestACL(own), true) {
		t.Fatal("foreign owner admitted")
	}
	for _, sid := range [][]byte{other, admins} {
		for _, mask := range []uint32{1, 2, 4, 0x10000, 0x40000, 0x80000, 0x80000000, 0x40000000} {
			for _, flags := range []byte{0, 0x10} {
				if desktopControlACLPrivate(user, user, controlTestACL(own, controlTestACE{flags: flags, mask: mask, sid: sid}), true) {
					t.Fatalf("foreign effective grant admitted: mask=%x flags=%x", mask, flags)
				}
			}
		}
	}
	if !desktopControlACLPrivate(user, user, controlTestACL(own, controlTestACE{flags: 8, mask: 1, sid: other}), true) {
		t.Fatal("inherit-only read grants no current access")
	}
	if desktopControlACLPrivate(user, user, controlTestACL(own, controlTestACE{kind: 1, mask: 1, sid: other}, controlTestACE{mask: 1, sid: other}), true) {
		t.Fatal("deny must not excuse unsafe allow")
	}
	for _, mask := range []uint32{2, 4, 0x40, 0x10000, 0x40000, 0x80000} {
		if desktopControlACLPrivate(user, user, controlTestACL(own, controlTestACE{mask: mask, sid: other}), false) {
			t.Fatalf("directory replacement/control grant admitted: %x", mask)
		}
	}
}

func TestWindowsControlPrivacyRejectsMissingAndUnsupportedEvidence(t *testing.T) {
	user := []byte{1, 1, 0, 0, 0, 0, 0, 5, 21, 0, 0, 0}
	good := controlTestACL(controlTestACE{mask: 0x1F01FF, sid: user})
	for length := range len(good) {
		if desktopControlACLPrivate(user, user, good[:length], true) {
			t.Fatalf("truncated ACL admitted at %d", length)
		}
	}
	for _, kind := range []byte{2, 5, 9, 255} {
		if desktopControlACLPrivate(user, user, controlTestACL(controlTestACE{kind: kind, mask: 1, sid: user}), true) {
			t.Fatalf("unsupported ACE type admitted: %d", kind)
		}
	}
	if desktopControlACLPrivate(user, user, controlTestACL(controlTestACE{mask: 0x200, sid: user}), true) {
		t.Fatal("unknown access right admitted")
	}
	good[9] = 0x80
	if desktopControlACLPrivate(user, user, good, true) {
		t.Fatal("unsupported flags admitted")
	}
}
