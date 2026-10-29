package main

import (
	"context"
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"errors"
	"os"
	"runtime"
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
	shell              nativeShellLayout
	onboardingStep     nativeOnboardingStep
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

var native = &nativeDesktop{store: model.NewNativeStore(), shell: newNativeShellLayout()}

func init() { native.wireStore() }

// wireStore binds fire-and-forget CLI methods (mark_read, …) to the admitted transport.
func (n *nativeDesktop) wireStore() {
	if n.store == nil {
		return
	}
	n.store.Perform = n.performCLI
}

func (n *nativeDesktop) performCLI(method string, params map[string]any) {
	payload, err := json.Marshal(params)
	if err != nil || !n.store.Connected || !n.store.HasIdentity {
		return
	}
	authority, client := n.authority, app.cli
	if client == nil {
		return
	}
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		_, _ = client.requestBound(ctx, method, payload, &authority)
	}()
}

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
		n.shell = newNativeShellLayout()
		syncNativeLanguage()
		options := mygo.WindowOptions{
			Title:          appName() + " — Native",
			Width:          1180,
			Height:         760,
			MinWidth:       860,
			MinHeight:      520,
			TitleBarStyle:  mygo.TitleBarHidden,
			TitleBarHeight: headerHeight,
			// Sidebar shows the window material (d94feac8); elsewhere Context.Vibrancy is false.
			Vibrancy: mygo.VibrancySidebar,
			Content:  ui.View(n.view),
		}
		if runtime.GOOS == "darwin" {
			options.TrafficLightPosition = &mygo.Point{X: 19, Y: 19}
		}
		n.win = mygo.NewWindow(options)
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
	n.shellView(c)
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
