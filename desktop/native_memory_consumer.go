package main

import "encoding/json"

func (m *nativeMemory) approvalPreview(id uint64) (json.RawMessage, error) {
	var out json.RawMessage
	err := m.runtime.eval(`const a=globalThis.nativeMemoryApprovals?.[input.id];if(!a)throw new Error("approval_not_found");return a.preview;`, map[string]any{"id": id}, &out)
	return out, err
}
func memoryOperationAllowed(health json.RawMessage, action string) bool {
	var h struct {
		Capabilities struct {
			Status bool `json:"operation_status"`
			Cancel bool `json:"cancel_operation"`
		} `json:"capabilities"`
	}
	if json.Unmarshal(health, &h) != nil {
		return false
	}
	switch action {
	case "status":
		return h.Capabilities.Status
	case "cancel":
		return h.Capabilities.Cancel
	}
	return false
}
