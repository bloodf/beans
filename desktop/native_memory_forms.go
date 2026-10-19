package main

import (
	"context"
	"encoding/json"
	"errors"
	"strconv"
	"time"

	"github.com/egoist/mygo/ui"
)

type memoryConnectionRow struct {
	ID           string          `json:"id"`
	Name         string          `json:"name"`
	Backend      string          `json:"backend"`
	Embedding    *string         `json:"embedding_profile"`
	Availability string          `json:"availability"`
	Revision     json.RawMessage `json:"revision"`
}
type memoryBindingForm struct {
	Bot, Mode, Account, User, Secret, SecretMode string
	Remove                                       bool
}
type memoryConnectionForm struct {
	ID, Name, Backend, Endpoint, EndpointMode, Secret, SecretMode, Embedding, Schema, Role, Region string
	Options                                                                                        bool
	Bindings                                                                                       []memoryBindingForm
	ReplaceAll, ConfirmReplace                                                                     bool
	Existing                                                                                       bool
	Disconnect                                                                                     bool
}
type memoryAssetForm struct{ Source, Location, License, Bytes, SHA string }
type memoryBotForm struct {
	ID, Connection                         string
	Recall, Capture, Group, Unattended     bool
	Cap                                    string
	Timeout, Bytes, Results, Context       string
	Draft, View                            uint64
	Saved                                  json.RawMessage
	Dirty                                  bool
	Health, Operations                     json.RawMessage
	Document                               string
	DeleteConfirm                          bool
	OperationCancel                        bool
	Profile, ProfileCounter, ProfileDevice string
	Assets                                 [3]memoryAssetForm
	SetupMethod, Path                      string
	Create, Confirm                        bool
	Approval                               uint64
	Preview, Target                        json.RawMessage
	Recovering                             bool
}
type memoryForms struct {
	Connection *memoryConnectionForm
	Bot        *memoryBotForm
	Embedding  *memoryEmbeddingForm
	Note       *memoryNoteForm
	Pending    bool
	Error      string
	serial     uint64
}

// Every completion is ordered, account-bound and form-generation-bound. Cancel
// closes local drafts only; admitted mutations cannot be recalled by dismissal.
func (n *nativeDesktop) memoryCall(method string, params json.RawMessage, done func(json.RawMessage, error)) {
	f := &n.memoryPanel.Forms
	if f.Pending {
		return
	}
	if !n.store.Connected {
		f.Error = "Wait for account admission"
		return
	}
	f.Pending = true
	f.Error = ""
	serial, epoch, authority := f.serial, n.store.Epoch, n.authority
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		data, err := n.memoryRequest(ctx, method, params, authority)
		postMain(func() {
			if epoch != n.store.Epoch || serial != n.memoryPanel.Forms.serial {
				return
			}
			n.memoryPanel.Forms.Pending = false
			done(data, err)
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func jsonBytes(v any) json.RawMessage { b, _ := json.Marshal(v); return b }
func (n *nativeDesktop) editMemoryConnection(row *memoryConnectionRow) {
	f := &n.memoryPanel.Forms
	f.serial++
	f.Error = ""
	f.Connection = &memoryConnectionForm{ID: "memory-" + strconv.FormatInt(time.Now().UnixNano(), 10), Backend: "hindsight", EndpointMode: "replace", SecretMode: "keep"}
	if row != nil {
		f.Connection.Existing = true
		f.Connection.ID = row.ID
		f.Connection.Name = row.Name
		f.Connection.Backend = row.Backend
		f.Connection.EndpointMode = "keep"
		if row.Embedding != nil {
			f.Connection.Embedding = *row.Embedding
		}
	}
}
func (n *nativeDesktop) saveMemoryConnection() {
	f := &n.memoryPanel.Forms
	d := f.Connection
	if d == nil {
		return
	}
	edit := map[string]any{"id": d.ID, "name": d.Name, "backend": d.Backend, "secret": map[string]any{"action": d.SecretMode, "value": d.Secret}, "embedding_profile": nil}
	if d.Embedding != "" {
		edit["embedding_profile"] = d.Embedding
	}
	if d.EndpointMode == "clear" {
		edit["endpoint"] = nil
	} else if d.EndpointMode == "replace" {
		edit["endpoint"] = d.Endpoint
	}
	if d.Options {
		switch d.Backend {
		case "hindsight":
			edit["options"] = map[string]any{"backend": d.Backend}
		case "pgvector":
			edit["options"] = map[string]any{"backend": d.Backend, "schema": d.Schema, "role": d.Role}
		case "lance_db":
			edit["options"] = map[string]any{"backend": d.Backend, "region": d.Region}
		case "open_viking":
			bindings := map[string]any{}
			for _, b := range d.Bindings {
				if b.Bot == "" {
					f.Error = "Choose a bot for each binding"
					return
				}
				if _, ok := bindings[b.Bot]; ok {
					f.Error = "Choose distinct bots"
					return
				}
				if b.Remove {
					bindings[b.Bot] = nil
				} else {
					bindings[b.Bot] = map[string]any{"mode": b.Mode, "account_id": b.Account, "user_id": b.User, "secret": map[string]any{"action": b.SecretMode, "value": b.Secret}}
				}
			}
			edit["options"] = map[string]any{"backend": "open_viking", "bindings": bindings, "replace_all": d.ReplaceAll, "confirm_replace_all": d.ConfirmReplace}
		}
	}
	params, err := n.memory.connection(jsonBytes(edit))
	if err != nil {
		f.Error = err.Error()
		return
	}
	n.memoryCall("memory.connections.set", params, func(_ json.RawMessage, err error) {
		if err != nil {
			f.Error = err.Error()
			return
		}
		d.Secret = ""
		f.Connection = nil
		n.loadMemory()
	})
}
func (n *nativeDesktop) openBotMemory(id string) {
	f := &n.memoryPanel.Forms
	f.serial++
	f.Error = ""
	f.Bot = &memoryBotForm{ID: id, Cap: "1", Timeout: "2000", Bytes: "16384", Results: "8", Context: "4000", SetupMethod: "memory.pgvector.initialize.preview"}
	n.refreshBotMemory(true)
}
func (n *nativeDesktop) refreshBotMemory(rebase bool) {
	f := &n.memoryPanel.Forms
	b := f.Bot
	if b == nil {
		return
	}
	n.memoryCall("memory.preferences.get", jsonBytes(map[string]string{"bot_id": b.ID}), func(data json.RawMessage, err error) {
		if err == nil {
			data, err = n.memory.masked("preferences", data)
		}
		if err != nil {
			f.Error = err.Error()
			return
		}
		if b.View != 0 {
			_, err = n.memory.deletion(b.View, "refresh", data)
			if err != nil {
				f.Error = err.Error()
				return
			}
		}
		if b.Recovering && !rebase {
			b.Draft, err = n.memory.newDraft(b.ID, nil)
			if err == nil {
				b.View, err = n.memory.newView(b.ID, data)
			}
			if err != nil {
				f.Error = err.Error()
				return
			}
			b.Recovering = false
			if err = n.syncMemoryConsent(); err != nil {
				f.Error = err.Error()
			}
		}
		if rebase || b.Draft == 0 {
			b.Draft, err = n.memory.newDraft(b.ID, data)
			if err == nil {
				b.View, err = n.memory.newView(b.ID, data)
			}
			if err != nil {
				f.Error = err.Error()
				return
			}
			var p struct {
				Connection *string `json:"connection_id"`
				Recall     bool    `json:"auto_recall"`
				Capture    bool    `json:"capture_conversation"`
				Group      bool    `json:"capture_group_text"`
				Unattended bool    `json:"unattended_capture"`
				Cap        int     `json:"max_capture_deliveries_per_turn"`
				Budget     struct {
					Timeout int `json:"timeout_ms"`
					Bytes   int `json:"max_bytes"`
					Results int `json:"max_results"`
					Context int `json:"max_context_chars"`
				} `json:"recall_budget"`
			}
			_ = json.Unmarshal(data, &p)
			b.Connection = ""
			if p.Connection != nil {
				b.Connection = *p.Connection
			}
			b.Recall = p.Recall
			b.Capture = p.Capture
			b.Group = p.Group
			b.Unattended = p.Unattended
			b.Cap = strconv.Itoa(p.Cap)
			b.Timeout = strconv.Itoa(p.Budget.Timeout)
			b.Bytes = strconv.Itoa(p.Budget.Bytes)
			b.Results = strconv.Itoa(p.Budget.Results)
			b.Context = strconv.Itoa(p.Budget.Context)
			b.Dirty = false
		}
		b.Saved = data
		n.memoryCall("memory.operations.list", jsonBytes(map[string]string{"bot_id": b.ID}), func(data json.RawMessage, err error) {
			if err == nil {
				b.Operations, err = n.memory.masked("operations", data)
			}
			if err != nil {
				f.Error = err.Error()
			}
		})
	})
}
func (n *nativeDesktop) syncMemoryConsent() error {
	b := n.memoryPanel.Forms.Bot
	if b == nil {
		return errors.New("No bot draft")
	}
	values := map[string]any{"autoRecall": b.Recall, "captureConversation": b.Capture, "captureGroupText": b.Group, "unattendedCapture": b.Unattended}
	var connection any
	if b.Connection != "" {
		connection = b.Connection
	}
	values["connection"] = connection
	for action, value := range values {
		if _, err := n.memory.draft(b.Draft, action, value); err != nil {
			return err
		}
	}
	cap, err := strconv.Atoi(b.Cap)
	if err != nil {
		return err
	}
	if _, err = n.memory.draft(b.Draft, "cap", cap); err != nil {
		return err
	}
	budget := map[string]int{}
	for key, text := range map[string]string{"timeout_ms": b.Timeout, "max_bytes": b.Bytes, "max_results": b.Results, "max_context_chars": b.Context} {
		value, err := strconv.Atoi(text)
		if err != nil {
			return err
		}
		budget[key] = value
	}
	_, err = n.memory.draft(b.Draft, "budget", budget)
	return err
}
func (n *nativeDesktop) saveMemoryConsent() {
	f := &n.memoryPanel.Forms
	b := f.Bot
	if err := n.syncMemoryConsent(); err != nil {
		f.Error = err.Error()
		return
	}
	request, err := n.memory.draft(b.Draft, "request", nil)
	if err != nil {
		f.Error = err.Error()
		return
	}
	var rpc struct {
		Params json.RawMessage `json:"params"`
	}
	_ = json.Unmarshal(request, &rpc)
	for _, row := range n.memoryRows() {
		if row.ID == b.Connection && row.Availability == "blocked" {
			f.Error = "Connection is blocked"
			return
		}
	}
	n.memoryCall("memory.preferences.set", rpc.Params, func(_ json.RawMessage, err error) {
		if err != nil {
			f.Error = err.Error()
			return
		}
		n.refreshBotMemory(true)
	})
}
func (n *nativeDesktop) memoryRows() []memoryConnectionRow {
	var data struct {
		Connections []memoryConnectionRow `json:"connections"`
	}
	_ = json.Unmarshal(n.memoryPanel.Connections, &data)
	return data.Connections
}
func memoryField(c *ui.Context, label string, value *string) {
	ui.TextInput(c, value).Label(label)
}
func (n *nativeDesktop) memoryFormView(c *ui.Context) {
	f := &n.memoryPanel.Forms
	ui.Text(c, "Memory settings").Bold()
	if f.Error != "" {
		ui.Text(c, f.Error)
	}
	ui.Scroll(c).Grow(1).Children(func() {
		ui.Column(c).Gap(8).Disabled(f.Pending).Children(func() {
			if d := f.Connection; d != nil {
				memoryField(c, "Connection name", &d.Name)
				ui.Select(c, &d.Backend, []string{"hindsight", "pgvector", "lance_db", "open_viking"}).Label("Backend").Disabled(d.Existing)
				ui.Select(c, &d.EndpointMode, []string{"keep", "replace", "clear"}).Label("Endpoint change")
				if d.EndpointMode == "replace" {
					memoryField(c, "Exact endpoint", &d.Endpoint)
				}
				ui.Select(c, &d.SecretMode, []string{"keep", "replace", "clear"}).Label("Secret change")
				if d.SecretMode == "replace" {
					ui.TextInput(c, &d.Secret).Password().Label("Replacement secret")
				}
				memoryField(c, "Embedding profile ID", &d.Embedding)
				ui.Checkbox(c, &d.Options, "Replace backend settings")
				if d.Options && d.Backend == "pgvector" {
					memoryField(c, "Schema", &d.Schema)
					memoryField(c, "Role", &d.Role)
				}
				if d.Options && d.Backend == "lance_db" {
					memoryField(c, "Region", &d.Region)
				}
				if d.Options && d.Backend == "open_viking" {
					ui.Checkbox(c, &d.ReplaceAll, "Replace all bindings")
					if d.ReplaceAll {
						ui.Checkbox(c, &d.ConfirmReplace, "Confirm removal of omitted bindings")
					}
					for i := range d.Bindings {
						b := &d.Bindings[i]
						ui.Column(c.Key(strconv.Itoa(i))).Gap(4).Children(func() {
							memoryField(c, "Binding bot ID", &b.Bot)
							ui.Checkbox(c, &b.Remove, "Remove binding")
							if !b.Remove {
								ui.Select(c, &b.Mode, []string{"user_key", "trusted_gateway"}).Label("Binding mode")
								memoryField(c, "Binding account", &b.Account)
								memoryField(c, "Binding user", &b.User)
								ui.Select(c, &b.SecretMode, []string{"keep", "replace", "clear"}).Label("Binding secret change")
								if b.SecretMode == "replace" {
									ui.TextInput(c, &b.Secret).Password().Label("Binding replacement secret")
								}
							}
						})
					}
					if ui.Button(c, "Add bot binding").Clicked() {
						d.Bindings = append(d.Bindings, memoryBindingForm{Mode: "user_key", SecretMode: "keep"})
					}
				}
			} else if b := f.Bot; b != nil {
				n.botMemoryControls(c, b)
			}
		})
	})
	if f.Pending {
		ui.Text(c, "Request pending…")
	}
	ui.Row(c).Gap(8).Children(func() {
		if ui.Button(c, "Cancel memory editing").Disabled(f.Pending).Clicked() {
			f.serial++
			f.Connection = nil
			f.Bot = nil
			f.Error = ""
		}
		if f.Connection != nil && !f.Connection.Disconnect && ui.Button(c, "Save connection").Disabled(f.Pending).Clicked() {
			n.saveMemoryConnection()
		}
		if f.Connection != nil && f.Connection.Disconnect && ui.Button(c, "Confirm disconnect").Disabled(f.Pending).Clicked() {
			id := f.Connection.ID
			n.memoryCall("memory.connections.disconnect", jsonBytes(map[string]string{"id": id}), func(_ json.RawMessage, err error) {
				if err != nil {
					f.Error = err.Error()
				} else {
					f.Connection = nil
					n.loadMemory()
				}
			})
		}
		if f.Bot != nil && ui.Button(c, "Save consent").Disabled(f.Pending).Clicked() {
			n.saveMemoryConsent()
		}
	})
}

func (n *nativeDesktop) confirmMemoryDisconnect(row memoryConnectionRow) {
	f := &n.memoryPanel.Forms
	f.serial++
	f.Connection = &memoryConnectionForm{ID: row.ID, Name: row.Name, Backend: row.Backend, Existing: true, Disconnect: true, EndpointMode: "keep", SecretMode: "keep"}
	f.Error = "Disconnect stops bots using this connection; it does not erase remote memory."
}
