package main

import (
	"bytes"
	"encoding/binary"
)

// Windows control objects trust the current account and LocalSystem only. Group
// membership (including Administrators) never establishes token confidentiality.
func desktopControlSID(sid []byte) bool {
	return len(sid) >= 8 && sid[0] == 1 && sid[1] <= 15 && len(sid) == 8+4*int(sid[1])
}

func desktopControlACLPrivate(owner, user, acl []byte, private bool) bool {
	system := []byte{1, 1, 0, 0, 0, 0, 0, 5, 18, 0, 0, 0}
	trusted := func(sid []byte) bool { return bytes.Equal(sid, user) || bytes.Equal(sid, system) }
	if !desktopControlSID(owner) || !desktopControlSID(user) || !trusted(owner) || len(acl) < 8 ||
		(acl[0] != 2 && acl[0] != 4) || int(binary.LittleEndian.Uint16(acl[2:4])) != len(acl) {
		return false
	}
	// Directory/marker integrity includes replacement, ownership and ACL changes.
	unsafeRights := uint32(0x000D0156 | 0x50000000 | 0x03000000)
	if private {
		// Token confidentiality also excludes data/EA reads and execute grants.
		unsafeRights |= 0x00000029 | 0xA0000000
	}
	position := 8
	for count := int(binary.LittleEndian.Uint16(acl[4:6])); count > 0; count-- {
		if position+4 > len(acl) {
			return false
		}
		ace := acl[position:]
		size := int(binary.LittleEndian.Uint16(ace[2:4]))
		if size < 16 || size > len(ace) || ace[0] > 1 || ace[1]&^byte(0x1F) != 0 {
			return false
		}
		ace = ace[:size]
		sid := ace[8:]
		if !desktopControlSID(sid) {
			return false
		}
		mask := binary.LittleEndian.Uint32(ace[4:8])
		if mask&^uint32(0xF31F01FF) != 0 {
			return false
		}
		// INHERIT_ONLY does not grant access on this object; inherited effective
		// allows are checked exactly like explicit ones. Denies never excuse an allow.
		if ace[0] == 0 && ace[1]&0x08 == 0 && !trusted(sid) && mask&unsafeRights != 0 {
			return false
		}
		position += size
	}
	return position <= len(acl)
}
