package main

import (
	"encoding/json"
	"errors"
)

// Validate the same concrete result shapes as MemorySetupAPI.apply; no success
// is inferred from an empty reply or from an installation without runtime.
func (m *nativeMemory) setupResult(method string, data json.RawMessage) error {
	var r struct {
		Installed   bool   `json:"installed"`
		Initialized bool   `json:"initialized"`
		Status      string `json:"status"`
		Exported    bool   `json:"exported"`
		Imported    bool   `json:"imported"`
		Bytes       *int64 `json:"bytes"`
	}
	if err := json.Unmarshal(data, &r); err != nil {
		return err
	}
	valid := false
	switch method {
	case "memory.embeddings.local.apply":
		valid = r.Installed && (r.Status == "ready" || r.Status == "runtime_unavailable")
	case "memory.pgvector.initialize.apply":
		valid = r.Initialized
	case "memory.lance.binding.apply":
		valid = r.Status == "ready"
	case "memory.lance.export.apply":
		valid = r.Exported && r.Bytes != nil && *r.Bytes >= 0 && *r.Bytes <= 8*1024*1024
	case "memory.lance.import.apply":
		valid = r.Imported
	}
	if !valid {
		return errors.New("invalid_setup_reply")
	}
	return nil
}
