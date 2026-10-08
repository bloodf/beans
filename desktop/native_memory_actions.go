package main

import (
	"encoding/json"
	"strconv"
	"time"

	"github.com/egoist/mygo/ui"
)

func (n *nativeDesktop) botMemoryControls(c *ui.Context, b *memoryBotForm) {
	f := &n.memoryPanel.Forms
	connections := []string{""}
	for _, row := range n.memoryRows() {
		if row.Availability == "supported" || row.ID == b.Connection {
			connections = append(connections, row.ID)
		}
	}
	if ui.Select(c, &b.Connection, connections).Label("Bot memory connection").Changed() {
		b.Dirty = true
		_ = n.syncMemoryConsent()
		b.Approval = 0
		b.Confirm = false
	}
	for _, item := range []struct {
		label string
		value *bool
	}{{"Auto recall", &b.Recall}, {"Capture future conversations", &b.Capture}, {"Capture group text", &b.Group}, {"Unattended capture", &b.Unattended}} {
		if ui.Checkbox(c, item.value, item.label).Changed() {
			b.Dirty = true
			_ = n.syncMemoryConsent()
		}
	}
	for _, item := range []struct {
		label string
		value *string
	}{{"Capture deliveries per turn", &b.Cap}, {"Recall timeout ms", &b.Timeout}, {"Recall max bytes", &b.Bytes}, {"Recall max results", &b.Results}, {"Recall context characters", &b.Context}} {
		memoryField(c, item.label, item.value)
	}
	ui.Text(c, "Remote memory receives approved plaintext. Group capture needs separate approval. Unattended capture is unavailable.")
	if ui.Button(c, "Approve exact plaintext choices").Clicked() {
		if err := n.syncMemoryConsent(); err != nil {
			f.Error = err.Error()
		} else {
			_, err = n.memory.draft(b.Draft, "approvePlaintext", nil)
			if err != nil {
				f.Error = err.Error()
			}
		}
	}
	if ui.Button(c, "Approve exact group capture").Clicked() {
		if err := n.syncMemoryConsent(); err != nil {
			f.Error = err.Error()
		} else {
			_, err = n.memory.draft(b.Draft, "approveGroup", nil)
			if err != nil {
				f.Error = err.Error()
			}
		}
	}
	if ui.Button(c, "Reload authoritative status").Clicked() {
		n.refreshBotMemory(false)
	}
	if ui.Button(c, "Check remote service").Clicked() {
		n.memoryCall("memory.service.health", jsonBytes(map[string]string{"bot_id": b.ID}), func(data json.RawMessage, err error) {
			if err == nil {
				b.Health, err = n.memory.masked("health", data)
			}
			if err != nil {
				f.Error = err.Error()
			}
		})
	}
	if len(b.Health) > 0 {
		ui.Text(c, string(b.Health))
	}
	if len(b.Operations) > 0 {
		ui.Text(c, string(b.Operations))
		var operations struct {
			Operations []struct {
				ID    string `json:"id"`
				State string `json:"state"`
			} `json:"operations"`
		}
		_ = json.Unmarshal(b.Operations, &operations)
		for _, operation := range operations.Operations {
			ui.Row(c.Key(operation.ID)).Gap(6).Children(func() {
				ui.Text(c, operation.ID+" · "+operation.State)
				if ui.Button(c, "Operation status").Clicked() {
					n.memoryOperation(operation.ID, "status")
				}
				if ui.Button(c, "Cancel operation…").Clicked() {
					b.Document = operation.ID
					b.OperationCancel = true
				}
			})
		}
	}
	if b.OperationCancel {
		ui.Text(c, "Cancellation may not prevent late writes; deletion remains pending until verified.")
		if ui.Button(c, "Keep operation").Clicked() {
			b.OperationCancel = false
		}
		if ui.Button(c, "Confirm cancel operation").Clicked() {
			b.OperationCancel = false
			n.memoryOperation(b.Document, "cancel")
		}
	}
	ui.Select(c, &b.SetupMethod, []string{"memory.pgvector.initialize.preview", "memory.lance.binding.preview", "memory.lance.export.preview", "memory.lance.import.preview", "memory.embeddings.local.preview"}).Label("Setup action")
	memoryField(c, "Exact setup path or directory", &b.Path)
	ui.Checkbox(c, &b.Create, "Create local Lance binding")
	if b.SetupMethod == "memory.embeddings.local.preview" {
		memoryField(c, "Profile ID", &b.Profile)
		memoryField(c, "Profile revision counter", &b.ProfileCounter)
		memoryField(c, "Profile revision device", &b.ProfileDevice)
		for i, kind := range []string{"runtime", "model", "tokenizer"} {
			a := &b.Assets[i]
			ui.Text(c, kind)
			ui.Select(c, &a.Source, []string{"supplied", "download"}).Label(kind + " source")
			memoryField(c, kind+" exact path or HTTPS URL", &a.Location)
			memoryField(c, kind+" license", &a.License)
			memoryField(c, kind+" bytes", &a.Bytes)
			memoryField(c, kind+" SHA-256", &a.SHA)
		}
	}
	if ui.Button(c, "Preview exact setup").Clicked() {
		for _, row := range n.memoryRows() {
			if row.ID == b.Connection && row.Availability == "blocked" {
				f.Error = "Connection is blocked; setup is unavailable"
				return
			}
		}
		n.previewMemorySetup()
	}
	if b.Approval != 0 {
		ui.Text(c, string(b.Preview))
		ui.Checkbox(c, &b.Confirm, "Confirm displayed setup target and effects")
		if ui.Button(c, "Apply one-use approval").Clicked() {
			n.applyMemorySetup()
		}
	}
	var health struct {
		Capabilities struct {
			Delete bool `json:"delete_document"`
			Clear  bool `json:"clear"`
		} `json:"capabilities"`
	}
	_ = json.Unmarshal(b.Health, &health)
	locked := true
	if b.View != 0 {
		v, err := n.memory.deletion(b.View, "locked", nil)
		locked = err != nil || string(v) == "true"
	}
	memoryField(c, "Document ID for deletion", &b.Document)
	ui.Checkbox(c, &b.DeleteConfirm, "Confirm deletion; pending is not verified erasure")
	if health.Capabilities.Delete && ui.Button(c, "Delete document").Disabled(locked || !b.DeleteConfirm || b.Document == "").Clicked() {
		n.deleteMemory(false)
	}
	if health.Capabilities.Clear && ui.Button(c, "Clear remote bank").Disabled(locked || !b.DeleteConfirm).Clicked() {
		n.deleteMemory(true)
	}
}
func (n *nativeDesktop) savedMemoryTarget() (json.RawMessage, json.RawMessage) {
	b := n.memoryPanel.Forms.Bot
	var prefs struct {
		Connection *string `json:"connection_id"`
	}
	_ = json.Unmarshal(b.Saved, &prefs)
	var revision json.RawMessage
	if prefs.Connection != nil {
		for _, row := range n.memoryRows() {
			if row.ID == *prefs.Connection {
				revision = row.Revision
				break
			}
		}
	}
	runner := ""
	for _, bot := range n.store.Bots {
		if bot.ID == b.ID {
			runner = bot.RunnerID
			break
		}
	}
	return jsonBytes(map[string]any{"bot_id": b.ID, "runner_id": runner, "connection_revision": revision}), revision
}
func (n *nativeDesktop) previewMemorySetup() {
	f := &n.memoryPanel.Forms
	b := f.Bot
	b.Approval = 0
	b.Confirm = false
	params := map[string]any{"bot_id": b.ID}
	switch b.SetupMethod {
	case "memory.lance.binding.preview":
		params["directory"] = b.Path
		params["create"] = b.Create
	case "memory.lance.export.preview", "memory.lance.import.preview":
		params["path"] = b.Path
	}
	if b.SetupMethod == "memory.embeddings.local.preview" {
		counter, err := strconv.Atoi(b.ProfileCounter)
		if err != nil {
			f.Error = err.Error()
			return
		}
		assets := []any{}
		for i, kind := range []string{"runtime", "model", "tokenizer"} {
			a := b.Assets[i]
			size, err := strconv.Atoi(a.Bytes)
			if err != nil {
				f.Error = err.Error()
				return
			}
			source := map[string]string{"kind": a.Source}
			if a.Source == "download" {
				source["url"] = a.Location
			} else {
				source["path"] = a.Location
			}
			assets = append(assets, map[string]any{"kind": kind, "source": source, "license": a.License, "bytes": size, "sha256": a.SHA})
		}
		var botRunner string
		for _, bot := range n.store.Bots {
			if bot.ID == b.ID {
				botRunner = bot.RunnerID
			}
		}
		params = map[string]any{"runner_id": botRunner, "profile_id": b.Profile, "profile_revision": map[string]any{"counter": counter, "device_id": b.ProfileDevice}, "plan": map[string]any{"assets": assets}}
	}
	request, err := n.memory.setupPreview(jsonBytes(map[string]any{"method": b.SetupMethod, "params": params}))
	if err != nil {
		f.Error = err.Error()
		return
	}
	var rpc struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	_ = json.Unmarshal(request, &rpc)
	target, _ := n.savedMemoryTarget()
	method := b.SetupMethod
	if method == "memory.embeddings.local.preview" {
		target = jsonBytes(map[string]any{"runner_id": params["runner_id"], "profile_id": params["profile_id"], "profile_revision": params["profile_revision"]})
	}
	n.memoryCall(rpc.Method, rpc.Params, func(data json.RawMessage, err error) {
		if err == nil {
			if err == nil && method == "memory.embeddings.local.preview" {
				var actual, expected struct {
					Runner   string `json:"runner_id"`
					Profile  string `json:"profile_id"`
					Revision struct {
						Counter int    `json:"counter"`
						Device  string `json:"device_id"`
					} `json:"profile_revision"`
				}
				if json.Unmarshal(data, &actual) != nil || json.Unmarshal(target, &expected) != nil || actual != expected {
					f.Error = "approval_stale"
					return
				}
				b.Approval, err = n.memory.setupApproval(method, data, time.Now().UnixMilli())
				if err != nil {
					f.Error = err.Error()
					return
				}
				b.Preview = data
				b.Target = target
				return
			}
			var preview struct {
				Bot      string          `json:"bot_id"`
				Runner   string          `json:"runner_id"`
				Revision json.RawMessage `json:"connection_revision"`
			}
			err = json.Unmarshal(data, &preview)
			if err == nil {
				var expected struct {
					Bot      string          `json:"bot_id"`
					Runner   string          `json:"runner_id"`
					Revision json.RawMessage `json:"connection_revision"`
				}
				_ = json.Unmarshal(target, &expected)
				if preview.Bot != expected.Bot || preview.Runner != expected.Runner || string(preview.Revision) != string(expected.Revision) {
					f.Error = "approval_stale"
					return
				}
				b.Approval, err = n.memory.setupApproval(method, data, time.Now().UnixMilli())
			}
		}
		if err != nil {
			f.Error = err.Error()
			return
		}
		b.Preview = data
		b.Target = target
	})
}
func (n *nativeDesktop) applyMemorySetup() {
	f := &n.memoryPanel.Forms
	b := f.Bot
	target, _ := n.savedMemoryTarget()
	if b.SetupMethod == "memory.embeddings.local.preview" {
		counter, err := strconv.Atoi(b.ProfileCounter)
		if err != nil {
			f.Error = err.Error()
			return
		}
		runner := ""
		for _, bot := range n.store.Bots {
			if bot.ID == b.ID {
				runner = bot.RunnerID
			}
		}
		target = jsonBytes(map[string]any{"runner_id": runner, "profile_id": b.Profile, "profile_revision": map[string]any{"counter": counter, "device_id": b.ProfileDevice}})
	}
	request, err := n.memory.setupApply(b.Approval, b.Confirm, target, time.Now().UnixMilli())
	if err != nil {
		f.Error = err.Error()
		return
	}
	b.Approval = 0
	b.Confirm = false
	var rpc struct {
		Method string          `json:"method"`
		Params json.RawMessage `json:"params"`
	}
	_ = json.Unmarshal(request, &rpc)
	n.memoryCall(rpc.Method, rpc.Params, func(data json.RawMessage, err error) {
		if err == nil {
			err = n.memory.setupResult(rpc.Method, data)
		}
		if err != nil {
			f.Error = err.Error()
		} else {
			n.refreshBotMemory(false)
		}
	})
}
func (n *nativeDesktop) deleteMemory(all bool) {
	f := &n.memoryPanel.Forms
	b := f.Bot
	if !b.DeleteConfirm {
		return
	}
	epoch, err := n.memory.deletion(b.View, "begin", nil)
	if err != nil {
		f.Error = err.Error()
		return
	}
	_, revision := n.savedMemoryTarget()
	params := map[string]any{"bot_id": b.ID, "confirm": true, "connection_revision": revision, "deletion_epoch": epoch}
	if !all {
		params["document_id"] = b.Document
	}
	b.DeleteConfirm = false
	n.memoryCall("memory.service.delete", jsonBytes(params), func(data json.RawMessage, err error) {
		if err != nil {
			f.Error = err.Error()
		}
		var reply struct {
			Epoch *int `json:"deletion_epoch"`
		}
		if json.Unmarshal(data, &reply) == nil && reply.Epoch != nil {
			if _, e := n.memory.deletion(b.View, "record", *reply.Epoch); e != nil {
				f.Error = e.Error()
			}
		}
		previous := f.Error
		n.refreshBotMemory(false)
		if previous != "" {
			f.Error = previous
		}
	})
}

func (n *nativeDesktop) memoryOperation(id, action string) {
	f := &n.memoryPanel.Forms
	b := f.Bot
	n.memoryCall("memory.operations."+action, jsonBytes(map[string]string{"bot_id": b.ID, "id": id}), func(_ json.RawMessage, err error) {
		if err != nil {
			f.Error = err.Error()
		} else {
			n.refreshBotMemory(false)
		}
	})
}
