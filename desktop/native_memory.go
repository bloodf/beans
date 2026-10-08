package main

import (
	"context"
	"encoding/json"
	"errors"
)

// Native memory uses the existing TS contract bundle, not duplicated consent,
// secret, masking or setup rules. Handles stay inside one account epoch.
type nativeMemory struct {
	runtime *sharedRuntime
	next    uint64
	epoch   uint64
}

var memoryMethods = map[string]bool{
	"memory.connections.list": true, "memory.connections.set": true, "memory.connections.disconnect": true,
	"memory.embeddings.set": true, "memory.embeddings.remove": true, "memory.preferences.get": true, "memory.preferences.set": true,
	"memory.service.health": true, "memory.service.recall": true, "memory.service.retain": true, "memory.service.inspect": true, "memory.service.reflect": true, "memory.service.advanced": true,
	"memory.operations.list": true, "memory.operations.retry": true, "memory.operations.status": true, "memory.operations.cancel": true, "memory.service.delete": true,
	"memory.embeddings.local.preview": true, "memory.embeddings.local.apply": true, "memory.embeddings.local.status": true,
	"memory.pgvector.initialize.preview": true, "memory.pgvector.initialize.apply": true, "memory.lance.binding.preview": true, "memory.lance.binding.apply": true,
	"memory.lance.export.preview": true, "memory.lance.export.apply": true, "memory.lance.import.preview": true, "memory.lance.import.apply": true,
}

func (n *nativeDesktop) memoryRequest(ctx context.Context, method string, params json.RawMessage, authority cliAuthority) (json.RawMessage, error) {
	if !memoryMethods[method] {
		return nil, errors.New("Unsupported memory request")
	}
	if isMock() {
		return nil, errors.New("Memory services are unavailable in demo mode.")
	}
	return app.cli.requestBound(ctx, method, params, &authority)
}
func (m *nativeMemory) masked(kind string, data json.RawMessage) (json.RawMessage, error) {
	readers := map[string]string{"connections": "readMemoryConnections", "preferences": "readMemoryPreferences", "health": "readMemoryHealth", "operations": "readMemoryOperations"}
	reader := readers[kind]
	if reader == "" {
		return nil, errors.New("Unsupported masked memory reply")
	}
	var out json.RawMessage
	err := m.runtime.eval("return BeansMemory."+reader+"(input);", data, &out)
	return out, err
}
func (m *nativeMemory) connection(edit json.RawMessage) (json.RawMessage, error) {
	var out struct {
		Params json.RawMessage `json:"params"`
	}
	err := m.runtime.eval("return BeansMemory.connectionRequest(input);", edit, &out)
	return out.Params, err
}
func (m *nativeMemory) newDraft(bot string, saved json.RawMessage) (uint64, error) {
	m.next++
	id := m.next
	if len(saved) == 0 {
		saved = json.RawMessage("null")
	}
	var result bool
	err := m.runtime.eval(`globalThis.nativeMemoryDrafts ??= {}; nativeMemoryDrafts[input.id] = new BeansMemory.MemoryPreferencesDraft(input.bot,input.saved ?? undefined); return true;`, map[string]any{"id": id, "bot": bot, "saved": saved}, &result)
	return id, err
}
func (m *nativeMemory) draft(id uint64, action string, value any) (json.RawMessage, error) {
	actions := map[string]string{"connection": "d.connectionID=input.value;", "autoRecall": "d.autoRecall=input.value;", "captureConversation": "d.captureConversation=input.value;", "captureGroupText": "d.captureGroupText=input.value;", "unattendedCapture": "d.unattendedCapture=input.value;", "cap": "d.maxCaptureDeliveriesPerTurn=input.value;", "budget": "d.recallBudget=input.value;", "approvePlaintext": "d.approveRemotePlaintext();", "approveGroup": "d.approveGroupCapture();", "request": "return d.request();"}
	code, ok := actions[action]
	if !ok {
		return nil, errors.New("Unsupported memory draft action")
	}
	var out json.RawMessage
	err := m.runtime.eval(`const d=globalThis.nativeMemoryDrafts?.[input.id]; if(!d)throw new Error("stale_memory_draft"); `+code+` return null;`, map[string]any{"id": id, "value": value}, &out)
	return out, err
}
func (m *nativeMemory) reset(epoch uint64) error {
	m.epoch = epoch
	var result bool
	return m.runtime.eval(`globalThis.nativeMemoryDrafts={};globalThis.nativeMemoryViews={};globalThis.nativeMemoryApprovals={};return true;`, nil, &result)
}
func (m *nativeMemory) setupPreview(request json.RawMessage) (json.RawMessage, error) {
	var out json.RawMessage
	err := m.runtime.eval(`return BeansMemory.setupPreviewRequest(input);`, request, &out)
	return out, err
}
func (m *nativeMemory) setupApproval(method string, data json.RawMessage, receivedMS int64) (uint64, error) {
	m.next++
	id := m.next
	var out bool
	err := m.runtime.eval(`globalThis.nativeMemoryApprovals ??= {}; const p = input.method === "memory.embeddings.local.preview" ? BeansMemory.readLocalAssetPreview(input.data) : BeansMemory.readBotSetupPreview(input.data,input.method); nativeMemoryApprovals[input.id] = input.method === "memory.embeddings.local.preview" ? new BeansMemory.LocalAssetApproval(p,input.now) : new BeansMemory.BotSetupApproval(p); return true;`, map[string]any{"id": id, "method": method, "data": data, "now": receivedMS}, &out)
	return id, err
}
func (m *nativeMemory) setupApply(id uint64, confirm bool, target json.RawMessage, nowMS int64) (json.RawMessage, error) {
	var out json.RawMessage
	err := m.runtime.eval(`const a=globalThis.nativeMemoryApprovals?.[input.id];if(!a)throw new Error("approval_not_found");return a.applyRequest(input.confirm,input.target,input.now);`, map[string]any{"id": id, "confirm": confirm, "target": target, "now": nowMS}, &out)
	return out, err
}
func (m *nativeMemory) newView(bot string, preferences json.RawMessage) (uint64, error) {
	m.next++
	id := m.next
	var out bool
	err := m.runtime.eval(`globalThis.nativeMemoryViews ??= {}; nativeMemoryViews[input.id]=new BeansMemory.MemoryServiceViewState(input.bot,BeansMemory.readMemoryPreferences(input.preferences));return true;`, map[string]any{"id": id, "bot": bot, "preferences": preferences}, &out)
	return id, err
}
func (m *nativeMemory) deletion(id uint64, action string, value any) (json.RawMessage, error) {
	actions := map[string]string{"begin": "return v.beginDeletion();", "record": "v.recordDeletionEpoch(input.value);return null;", "refresh": "v.refreshPreferences(BeansMemory.readMemoryPreferences(input.value));return null;", "locked": "return v.deletionLocked;"}
	code, ok := actions[action]
	if !ok {
		return nil, errors.New("Unsupported deletion action")
	}
	var out json.RawMessage
	err := m.runtime.eval(`const v=globalThis.nativeMemoryViews?.[input.id];if(!v)throw new Error("stale_memory_view");`+code, map[string]any{"id": id, "value": value}, &out)
	return out, err
}

func (m *nativeMemory) supportsAdvanced(capabilities json.RawMessage, feature, action string) (bool, error) {
	var out bool
	err := m.runtime.eval(`return BeansMemory.supportsMemoryAdvancedAction(input.capabilities,input.feature,input.action);`, map[string]any{"capabilities": capabilities, "feature": feature, "action": action}, &out)
	return out, err
}
func (m *nativeMemory) vectorEdit(documentID, text string) (json.RawMessage, error) {
	var out json.RawMessage
	err := m.runtime.eval(`return BeansMemory.memoryVectorEditBody(input.id,input.text);`, map[string]string{"id": documentID, "text": text}, &out)
	return out, err
}
