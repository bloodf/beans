package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"slices"
	"time"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/transfer"
	"github.com/egoist/mygo/ui"
)

// The native window shares the admitted account and sole CLI transport. The
// complete web client remains available until native feature parity is proven.
type nativeDesktop struct {
	store              *model.NativeStore
	win                *mygo.Window
	authority          cliAuthority
	avatars            *nativeAvatars
	activity           *nativeAvatarActivity
	memory             *nativeMemory
	memoryPanel        nativeMemoryPanel
	look               *nativeLookEditor
	memoryRecovery     map[string]memoryForms
	diagnostics        *nativeDiagnostics
	diagnosticsExports int
}

func nativeEnabled() bool { return os.Getenv("BEANS_NATIVE") == "1" }

var native = &nativeDesktop{store: model.NewNativeStore()}

func (n *nativeDesktop) event(name string, data json.RawMessage) {
	if name == "identity.changed" || name == "snapshot" {
		var v struct {
			Has     *bool  `json:"has_identity"`
			Account string `json:"identity_id"`
		}
		if json.Unmarshal(data, &v) == nil && v.Has != nil && (!*v.Has || (name == "snapshot" && n.store.AccountID != "" && v.Account != n.store.AccountID)) {
			n.suspendMemoryForms()
			n.resetDiagnostics()
		}
	}
	if err := n.store.Apply(name, data); err != nil {
		n.store.Error = err.Error()
	}
	if n.avatars != nil && n.avatars.epoch != n.store.Epoch {
		n.avatars.reset(n.store.Epoch)
		if n.activity != nil {
			n.activity.reset()
		}
		n.look = nil
	}
	if n.memory != nil && n.memory.epoch != n.store.Epoch {
		_ = n.memory.reset(n.store.Epoch)
		n.suspendMemoryForms()
	}
	if name == "snapshot" {
		n.recoverMemoryForms()
	}
	if n.activity != nil && n.store.Connected {
		if err := n.activity.event(name, data); err != nil {
			n.store.Error = err.Error()
		}
	}
	if n.win != nil {
		n.win.Invalidate()
	}
}
func (n *nativeDesktop) show() {
	if n.avatars == nil {
		r, err := newSharedRuntime()
		if err != nil {
			n.store.Error = err.Error()
			return
		}
		n.avatars = newNativeAvatars(r)
		n.memory = &nativeMemory{runtime: r}
		n.activity = newNativeAvatarActivity(r, func() {
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}
	if n.win == nil {
		n.win = mygo.NewWindow(mygo.WindowOptions{Title: "Beans — Native", Width: 1100, Height: 760, MinWidth: 640, MinHeight: 440, Content: ui.View(n.view)})
		visibility := func() {
			if n.avatars != nil {
				n.avatars.visible = n.win.IsVisible() && !n.win.IsMinimized()
				n.win.Invalidate()
			}
		}
		n.win.OnShow(visibility)
		n.win.OnHide(visibility)
		n.win.OnMinimize(visibility)
		n.win.OnRestore(visibility)
		n.win.OnClose(func(e *mygo.CloseEvent) {
			if n.diagnosticsExports > 0 {
				e.PreventDefault()
				return
			}
			if n.store.HasOwnedIntent() || n.hasMemoryIntent() || (n.look != nil && n.look.HasIntent()) {
				e.PreventDefault()
				n.store.Error = "Send or save native drafts before closing"
				n.win.Invalidate()
			}
		})
		n.win.OnClosed(func() { n.win = nil })
	}
	n.win.Show()
	n.win.Focus()
}

// Export writes draft text and attachment references to a user-selected private
// file. It never silently releases in-memory ownership or bypasses quit guards.
func (n *nativeDesktop) saveDrafts() {
	if !n.store.HasOwnedIntent() {
		return
	}
	for _, d := range n.store.Drafts {
		if d.Sending {
			n.store.Error = "Wait for the pending send before saving drafts"
			return
		}
	}
	data, err := json.Marshal(struct {
		Account  string
		Drafts   map[string]*model.NativeDraft
		Archived map[string]map[string]*model.NativeDraft
	}{n.store.AccountID, n.store.Drafts, n.store.ArchivedDrafts})
	if err != nil {
		n.store.Error = err.Error()
		return
	}
	epoch, win := n.store.Epoch, n.win
	go func() {
		path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: win, Title: "Save native drafts", DefaultPath: "beans-drafts.json"})
		if err == nil && path != "" {
			err = os.WriteFile(path, data, 0o600)
		}
		postMain(func() {
			if epoch != n.store.Epoch {
				return
			}
			if err != nil {
				n.store.Error = err.Error()
				return
			}
			if path == "" {
				return
			}
			// Export does not discard drafts: users can continue editing and the
			// close/quit veto remains until they explicitly discard saved intent.
			n.store.Error = "Drafts saved to " + path
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) confirmDiscard() {
	epoch, win := n.store.Epoch, n.win
	before, _ := json.Marshal(struct {
		Drafts   map[string]*model.NativeDraft
		Archived map[string]map[string]*model.NativeDraft
	}{n.store.Drafts, n.store.ArchivedDrafts})
	go func() {
		answer, err := mygo.Dialog.Message(mygo.MessageOptions{Parent: win, Message: "Discard all native drafts?", Detail: "This removes unsent text, attachment selections and replies, including drafts retained for another account. Save drafts first if you want to keep them.", Buttons: []string{"Keep drafts", "Discard drafts"}, CancelButton: 0})
		postMain(func() {
			if err != nil || answer.Button != 1 || epoch != n.store.Epoch {
				return
			}
			for _, d := range n.store.Drafts {
				if d.Sending {
					n.store.Error = "Wait for the pending send before discarding drafts"
					return
				}
			}
			after, _ := json.Marshal(struct {
				Drafts   map[string]*model.NativeDraft
				Archived map[string]map[string]*model.NativeDraft
			}{n.store.Drafts, n.store.ArchivedDrafts})
			if string(before) != string(after) {
				n.store.Error = "Drafts changed; review them before discarding"
				return
			}
			n.store.Drafts = map[string]*model.NativeDraft{}
			n.store.ArchivedDrafts = nil
			n.store.Error = ""
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) send(id string) {
	var bytes [16]byte
	if _, err := rand.Read(bytes[:]); err != nil {
		n.store.Draft(id).Error = err.Error()
		return
	}
	params, complete, err := n.store.BeginSend(id, "msg-"+hex.EncodeToString(bytes[:]))
	if err != nil {
		n.store.Draft(id).Error = err.Error()
		return
	}
	payload, err := json.Marshal(params)
	if err != nil {
		complete(err)
		return
	}
	authority, client := n.authority, app.cli
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_, err := client.requestBound(ctx, "chats.send", payload, &authority)
		postMain(func() {
			complete(err)
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) attach(id string) {
	epoch, win, draft := n.store.Epoch, n.win, n.store.Draft(id)
	go func() {
		paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Parent: win, Title: "Attach files", Multiple: true})
		files := make([]FileInfo, 0, len(paths))
		for _, path := range paths {
			files = append(files, inspect(path))
		}
		postMain(func() {
			if epoch != n.store.Epoch || win != n.win || n.store.Drafts[id] != draft {
				return
			}
			if err != nil {
				draft.Error = err.Error()
			}
			n.addFiles(draft, files)
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) addFiles(draft *model.NativeDraft, files []FileInfo) {
	for _, f := range files {
		if !f.IsFile || f.Size > 100*1024*1024 || len(draft.Attachments) >= 10 {
			draft.Error = "Attach at most 10 regular files, each no larger than 100 MB"
			continue
		}
		var raw [12]byte
		if _, err := rand.Read(raw[:]); err != nil {
			draft.Error = err.Error()
			break
		}
		draft.Attachments = append(draft.Attachments, model.NativeAttachment{ID: "att-" + hex.EncodeToString(raw[:]), Path: f.Path, Name: f.Name, Mime: f.Mime, Size: f.Size, Width: f.Width, Height: f.Height})
		draft.Revision++
	}
}
func (n *nativeDesktop) pasteImage(id string) bool {
	if mygo.Clipboard.ReadText() != "" {
		return false
	}
	data := mygo.Clipboard.ReadImage()
	if len(data) == 0 {
		return false
	}
	draft := n.store.Draft(id)
	if len(data) > 100*1024*1024 {
		draft.Error = "The pasted image is larger than 100 MB"
		return true
	}
	epoch, win := n.store.Epoch, n.win
	go func() {
		file, err := (Files{}).SavePasted("image.png", data)
		postMain(func() {
			if epoch != n.store.Epoch || win != n.win || n.store.Drafts[id] != draft {
				return
			}
			if err != nil {
				draft.Error = err.Error()
			} else {
				n.addFiles(draft, []FileInfo{file})
			}
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
	return true
}
func (n *nativeDesktop) stop(id string) {
	epoch := n.store.Epoch
	if !n.store.Connected {
		return
	}
	payload, err := json.Marshal(map[string]string{"chat_id": id})
	if err != nil {
		return
	}
	authority, client := n.authority, app.cli
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_, err := client.requestBound(ctx, "chats.stop", payload, &authority)
		postMain(func() {
			if epoch != n.store.Epoch || n.store.Chat(id) == nil {
				return
			}
			if err != nil {
				n.store.Draft(id).Error = err.Error()
			}
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) older(id string) {
	params, complete, err := n.store.BeginOlder(id)
	if err != nil {
		n.store.Error = err.Error()
		return
	}
	payload, err := json.Marshal(params)
	if err != nil {
		complete(nil, err)
		return
	}
	authority, client := n.authority, app.cli
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		data, err := client.requestBound(ctx, "chats.messages", payload, &authority)
		postMain(func() {
			complete(json.RawMessage(data), err)
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) view(c *ui.Context) {
	t := c.Theme()
	ui.Row(c).Fill().AlignItems(ui.Stretch).Children(func() {
		ui.Column(c).Width(240).Shrink(0).Padding(16).Gap(10).Background(t.Surface).Children(func() {
			ui.Text(c, "Beans").Bold().FontSize(20)
			ui.Scroll(c).Grow(1).Children(func() {
				if !n.store.Connected {
					return
				}
				for i := range n.store.Chats {
					chat := &n.store.Chats[i]
					if n.avatars != nil {
						ui.Box(c.Key("cluster:"+chat.ID)).Size(32, 32).Shrink(0).Children(func() {
							for _, slot := range nativeGroupAvatarSlots(chat.BotIDs, 32) {
								for _, b := range n.store.Bots {
									if b.ID == slot.BotID {
										ui.Box(c.Key(chat.ID+":"+b.ID)).Absolute().Left(slot.X).Top(slot.Y).Size(slot.Size, slot.Size).Children(func() {
											n.avatars.viewSlot(c, b, n.activity.state(b.ID, chat.ID), slot.Size, "sidebar:"+chat.ID+":"+b.ID)
										})
										n.fetchAvatar(b)
										break
									}
								}
							}
						})
					}
					if ui.Button(c.Key(chat.ID), n.store.Title(chat)).Selected(n.store.Selected == chat.ID).Clicked() {
						n.store.Selected = chat.ID
					}
				}
			})
			if ui.Button(c, "Open full client").Clicked() {
				app.showMainWindow()
			}
		})
		ui.Column(c).Grow(1).MinWidth(0).Padding(16).Gap(12).Children(func() {
			if n.look != nil {
				n.look.View(c)
				if n.look.Done() && n.look.CanDismiss() {
					n.look = nil
				}
				return
			}
			if n.store.HasOwnedIntent() && ui.Button(c, "Save native drafts").Clicked() {
				n.saveDrafts()
			}
			if len(n.memoryRecovery) > 0 {
				ui.Text(c, "Memory edits are retained for their original account. Reconnect that account to recover them.")
				if ui.Button(c, "Discard retained memory edits…").Clicked() {
					n.confirmDiscardMemoryRecovery()
				}
			}
			if n.store.Error != "" {
				ui.Text(c, n.store.Error)
			}
			if n.store.HasOwnedIntent() && ui.Button(c, "Discard native drafts…").Clicked() {
				n.confirmDiscard()
			}
			if !n.store.Connected {
				ui.Text(c, "The Beans CLI is not running")
				if ui.Button(c, "Reconnect").Clicked() {
					app.cli.reconnect()
				}
				return
			}
			if ui.Button(c, "Diagnostics…").Clicked() {
				n.openDiagnostics()
			}
			if n.diagnostics != nil {
				n.diagnostics.View(c)
				if n.diagnostics.Done() {
					n.diagnostics = nil
				}
				return
			}
			if n.memory != nil && ui.Button(c, "Memory connections").Clicked() {
				n.loadMemory()
			}
			if n.memoryPanel.Open {
				n.memoryView(c)
				return
			}
			if !n.store.HasIdentity {
				ui.Text(c, "Create or restore an identity in the full client.")
				return
			}
			chat := n.store.Chat(n.store.Selected)
			if chat == nil {
				ui.Text(c, "Select a chat")
				return
			}
			ui.Text(c, n.store.Title(chat)).Bold().FontSize(20)
			if n.avatars != nil && len(chat.BotIDs) > 1 {
				ui.Box(c.Key("header-cluster:"+chat.ID)).Size(32, 32).Children(func() {
					for _, slot := range nativeGroupAvatarSlots(chat.BotIDs, 32) {
						for _, bot := range n.store.Bots {
							if bot.ID == slot.BotID {
								ui.Box(c.Key(bot.ID)).Absolute().Left(slot.X).Top(slot.Y).Size(slot.Size, slot.Size).Children(func() {
									n.avatars.viewSlot(c, bot, n.activity.state(bot.ID, chat.ID), slot.Size, "header:"+chat.ID+":"+bot.ID)
								})
								n.fetchAvatar(bot)
								break
							}
						}
					}
				})
			}
			if n.store.Error != "" {
				ui.Text(c, n.store.Error)
			}
			ui.Scroll(c.Key(chat.ID + "-transcript")).Grow(1).Gap(12).Children(func() {
				if chat.HasMore && ui.Button(c, "Load older messages").Disabled(n.store.LoadingOlder[chat.ID]).Clicked() {
					n.older(chat.ID)
				}
				for _, m := range chat.Messages {
					ui.Column(c.Key(m.ID)).Gap(4).Children(func() {
						author := m.Author.Kind
						for _, b := range n.store.Bots {
							if b.ID == m.Author.BotID {
								author = b.Name
								break
							}
						}
						if n.avatars != nil && m.Author.Kind == "bot" {
							for _, bot := range n.store.Bots {
								if bot.ID == m.Author.BotID {
									n.avatars.viewSlot(c, bot, n.activity.state(bot.ID, chat.ID), 28, "message:"+chat.ID+":"+m.ID)
									n.fetchAvatar(bot)
									if ui.Button(c, "Edit bot Look").Clicked() {
										n.openLook(bot)
									}
									break
								}
							}
						}
						ui.Text(c, author).Bold()
						text := m.Body.Text
						if text == "" {
							text = m.Body.Summary
						}
						ui.Text(c, text)
						if m.State.Error != "" {
							ui.Text(c, m.State.Error)
						}
						if m.Body.Kind == "text" && m.State.Kind == "complete" && ui.Button(c, "Reply").Clicked() {
							d := n.store.Draft(chat.ID)
							d.ReplyTo = m.ID
							d.Revision++
						}
					})
				}
			})
			d := n.store.Draft(chat.ID)
			if d.ReplyTo != "" {
				ui.Text(c, "Replying to "+d.ReplyTo)
				if ui.Button(c, "Cancel reply").Clicked() {
					d.ReplyTo = ""
					d.Revision++
				}
			}
			for i, a := range d.Attachments {
				if ui.Button(c.Key(a.ID), "Remove "+a.Name).Clicked() {
					d.Attachments = append(d.Attachments[:i], d.Attachments[i+1:]...)
					d.Revision++
					break
				}
			}
			if ui.Button(c, "Attach files").Disabled(d.Sending).Clicked() {
				n.attach(chat.ID)
			}
			ui.Row(c).Gap(6).Wrap().Children(func() {
				for _, b := range n.store.Bots {
					picked := slices.Contains(d.Mentions, b.ID)
					label := "@" + b.Name
					if picked {
						label = "Remove " + label
					}
					if ui.Button(c.Key("mention-"+b.ID), label).Disabled(d.Sending).Clicked() {
						if picked {
							d.Mentions = slices.DeleteFunc(d.Mentions, func(id string) bool { return id == b.ID })
						} else {
							d.Mentions = append(d.Mentions, b.ID)
						}
						d.Revision++
					}
				}
			})
			input := ui.TextArea(c.Key(chat.ID+"-composer"), &d.Text).Lines(1, 8).Label("Message").Placeholder("Message…").Disabled(d.Sending)
			composing := input.Composing()
			input.HandleInput(func(ev ui.InputEvent) bool {
				if composing || ev.Kind != ui.InputKeyDown || ev.Key != ui.KeyEnter {
					return false
				}
				if (prefs.get().SendOnReturn && ev.Mods == 0) || (!prefs.get().SendOnReturn && ev.Mods == ui.Cmd) {
					n.send(chat.ID)
					return true
				}
				return false
			})
			if input.Changed() {
				d.Revision++
			}
			if d.Error != "" {
				ui.Text(c, d.Error)
			}
			for _, id := range n.store.Running {
				if id == chat.ID {
					if ui.Button(c, "Stop responding").Clicked() {
						n.stop(chat.ID)
					}
					break
				}
			}
			if ui.Button(c, "Send").Disabled(d.Sending).Clicked() {
				n.send(chat.ID)
			}
		})
	})
}

func nativeQuitAdmission() error {
	if native.diagnosticsExports > 0 {
		return errors.New("Wait for diagnostics export before quitting")
	}
	if native.store.HasOwnedIntent() || native.hasMemoryIntent() || (native.look != nil && native.look.HasIntent()) {
		return errors.New("Send or save native drafts before quitting")
	}
	return nil
}

func (n *nativeDesktop) resetDiagnostics() {
	if n.diagnostics != nil {
		n.diagnostics.Reset()
		n.diagnostics = nil
	}
}

func (n *nativeDesktop) openDiagnostics() {
	if !n.store.Connected || app == nil || app.cli == nil || n.diagnosticsExports > 0 {
		return
	}
	if n.diagnostics != nil {
		return
	}
	epoch, authority, client := n.store.Epoch, n.authority, app.cli
	var panel *nativeDiagnostics
	valid := func() bool { return n.store.Connected && n.store.Epoch == epoch && n.diagnostics == panel }
	panel = newNativeDiagnostics(func(method string, params json.RawMessage, done func(json.RawMessage, error)) {
		if !valid() || method != "diagnostics.report" {
			done(nil, errors.New("diagnostics unavailable"))
			return
		}
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			data, err := client.requestBound(ctx, method, params, &authority)
			postMain(func() {
				if valid() {
					done(json.RawMessage(data), err)
				}
			})
		}()
	}, func(report string) error {
		if !valid() {
			return errors.New("diagnostics unavailable")
		}
		return mygo.Clipboard.Write(transfer.TextData(report))
	}, func(report string, done func(error)) {
		if !valid() {
			done(errors.New("diagnostics unavailable"))
			return
		}
		parent := n.win
		n.diagnosticsExports++
		go func() {
			path, err := mygo.Dialog.Save(mygo.SaveDialogOptions{Parent: parent, Title: "Export diagnostics report", DefaultPath: "beans-diagnostics.json"})
			postMain(func() {
				if !valid() || err != nil || path == "" {
					n.diagnosticsExports--
					if valid() {
						done(err)
					}
					return
				}
				go func() {
					err := os.WriteFile(path, []byte(report), 0o600)
					postMain(func() {
						n.diagnosticsExports--
						if valid() {
							done(err)
						}
						if n.win != nil {
							n.win.Invalidate()
						}
					})
				}()
			})
		}()
	}, func() {
		if n.win != nil {
			n.win.Invalidate()
		}
	})
	n.diagnostics = panel
}
