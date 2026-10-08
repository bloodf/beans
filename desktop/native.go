package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"os"
	"time"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The native window shares the admitted account and sole CLI transport. The
// complete web client remains available until native feature parity is proven.
type nativeDesktop struct {
	store *model.NativeStore
	win   *mygo.Window
}

func nativeEnabled() bool { return os.Getenv("BEANS_NATIVE") == "1" }

var native = &nativeDesktop{store: model.NewNativeStore()}

func (n *nativeDesktop) event(name string, data json.RawMessage) {
	if err := n.store.Apply(name, data); err != nil {
		n.store.Error = err.Error()
	}
	if n.win != nil {
		n.win.Invalidate()
	}
}
func (n *nativeDesktop) show() {
	if n.win == nil {
		n.win = mygo.NewWindow(mygo.WindowOptions{Title: "Beans — Native", Width: 1100, Height: 760, MinWidth: 640, MinHeight: 440, Content: ui.View(n.view)})
		n.win.OnClosed(func() { n.win = nil })
	}
	n.win.Show()
	n.win.Focus()
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
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_, err := app.cli.request(ctx, "chats.send", payload)
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
				for i := range n.store.Chats {
					chat := &n.store.Chats[i]
					if ui.Button(c.Key(chat.ID), n.store.Title(chat)).Clicked() {
						n.store.Selected = chat.ID
					}
				}
			})
			if ui.Button(c, "Open full client").Clicked() {
				app.showMainWindow()
			}
		})
		ui.Column(c).Grow(1).MinWidth(0).Padding(16).Gap(12).Children(func() {
			if !n.store.Connected {
				ui.Text(c, "The Beans CLI is not running")
				if ui.Button(c, "Reconnect").Clicked() {
					app.cli.reconnect()
				}
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
			ui.Scroll(c.Key(chat.ID + "-transcript")).Grow(1).Gap(12).Children(func() {
				for _, m := range chat.Messages {
					ui.Column(c.Key(m.ID)).Gap(4).Children(func() {
						author := m.Author.Kind
						for _, b := range n.store.Bots {
							if b.ID == m.Author.BotID {
								author = b.Name
								break
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
			if ui.Button(c, "Send").Disabled(d.Sending).Clicked() {
				n.send(chat.ID)
			}
		})
	})
}
