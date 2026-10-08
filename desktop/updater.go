package main

import (
	"bytes"
	"context"
	"crypto/ed25519"
	"crypto/rand"
	"crypto/sha256"
	"encoding/base64"
	"encoding/hex"
	"encoding/json/jsontext"
	"encoding/json/v2"
	"errors"
	"fmt"
	"io"
	"log"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"time"

	"github.com/coder/websocket"
	"github.com/egoist/mygo"
)

// MyGo's plugin installs immediately and has no pre-install callback. Use its
// supported low-level updater and install only after an ordinary, guarded quit.
// Never call Relaunch: the next user launch runs the new version.
type UpdaterState struct {
	Version            string `json:"version"`
	LastCheck          int64  `json:"lastCheck"`
	AutomaticChecks    bool   `json:"automaticChecks"`
	AutomaticDownloads bool   `json:"automaticDownloads"`
}

var UpdaterChanged = mygo.NewEvent[UpdaterState]("updater:changed")

// Keep the plugin's existing file and field semantics, including saved opt-outs.
type updatePreferences struct {
	AutomaticChecks    *bool     `json:"automaticChecks,omitempty"`
	AutomaticDownloads bool      `json:"automaticDownloads,omitzero"`
	SkippedVersion     string    `json:"skippedVersion,omitzero"`
	LastCheck          time.Time `json:"lastCheck,omitzero"`
}

type beansUpdater struct {
	mu                       sync.Mutex
	prefs                    updatePreferences
	file                     string
	checking                 bool
	checkingQuit             bool
	quitApproved             bool
	installationQuitApproved bool
	pending                  *mygo.Update
	pendingRelease           *verifiedDesktopRelease
	pendingProtocol          int
	pendingManual            bool
	relayURL                 string
	lease                    *desktopUpdateLease
	finishing                bool
}

var desktopUpdater beansUpdater

func desktopUpdateTokenPath() string {
	if path, set := os.LookupEnv("BEANS_UPDATE_TOKEN_FILE"); set {
		return path
	}
	home, set := os.LookupEnv("BEANS_HOME")
	if !set {
		home = defaultCLIHome()
	}
	return filepath.Join(home, "update-token")
}

// Open only an existing, private, marked core home. Root keeps subsequent file
// operations bound to this directory even if its pathname changes.
func openDesktopTokenDirectory() (*os.Root, error) {
	home := filepath.Dir(desktopUpdateTokenPath())
	info, err := os.Lstat(home)
	if err != nil || !info.IsDir() || !desktopControlOwner(info) ||
		(runtime.GOOS != "windows" && info.Mode().Perm()&0o022 != 0) {
		return nil, errors.New("Desktop update control requires an existing private Beans v2 home")
	}
	root, err := os.OpenRoot(home)
	if err != nil {
		return nil, err
	}
	opened, err := root.Stat(".")
	if err != nil || !os.SameFile(info, opened) || !desktopControlOwner(opened) ||
		(runtime.GOOS != "windows" && opened.Mode().Perm()&0o022 != 0) {
		root.Close()
		return nil, errors.New("The Beans home changed before update-control admission")
	}
	data, err := readDesktopControlFile(root, "format.json", false)
	var marker struct { Format string `json:"format"` }
	if err != nil || json.Unmarshal(data, &marker) != nil || marker.Format != "beans-v2" {
		root.Close()
		return nil, errors.New("Desktop update control requires an established Beans v2 format marker")
	}
	return root, nil
}

func readDesktopControlFile(root *os.Root, name string, private bool) ([]byte, error) {
	info, err := root.Lstat(name)
	if err != nil || !info.Mode().IsRegular() || !desktopControlOwner(info) || info.Size() > 4096 ||
		(private && runtime.GOOS != "windows" && info.Mode().Perm()&0o077 != 0) {
		return nil, errors.New("Cannot read the private Beans update-control record")
	}
	file, err := openDesktopControlFile(root, name, os.O_RDONLY)
	if err != nil {
		return nil, err
	}
	defer file.Close()
	opened, err := file.Stat()
	if err != nil || !os.SameFile(info, opened) || !opened.Mode().IsRegular() || !desktopControlOwner(opened) || opened.Size() > 4096 ||
		(private && runtime.GOOS != "windows" && opened.Mode().Perm()&0o077 != 0) {
		return nil, errors.New("The Beans update-control record changed while opening")
	}
	data, err := io.ReadAll(io.LimitReader(file, 4097))
	if err != nil || len(data) > 4096 {
		return nil, errors.New("Cannot read the private Beans update-control record")
	}
	return data, nil
}

// Provision only the token for a CLI the launcher starts, never replace operator configuration.
func ensureDesktopUpdateToken() (string, error) {
	path := desktopUpdateTokenPath()
	if _, set := os.LookupEnv("BEANS_UPDATE_TOKEN_FILE"); set {
		return path, nil
	}
	root, err := openDesktopTokenDirectory()
	if err != nil {
		return "", err
	}
	defer root.Close()
	file, err := openDesktopControlFile(root, "update-token", os.O_WRONLY|os.O_CREATE|os.O_EXCL)
	if errors.Is(err, os.ErrExist) {
		data, readErr := readDesktopControlFile(root, "update-token", true)
		if readErr != nil || len(strings.TrimSpace(string(data))) < 32 {
			return "", errors.New("The existing update-control token is invalid; it is not replaced")
		}
		return path, nil
	}
	if err != nil {
		return "", err
	}
	// Secure only the new inode, before writing a secret; existing/operator ACLs
	// and Unix permissions are never changed.
	err = secureDesktopControlFile(file)
	var secret [32]byte
	if err == nil {
		_, err = rand.Read(secret[:])
	}
	if err == nil {
		_, err = file.WriteString(base64.RawURLEncoding.EncodeToString(secret[:]) + "\n")
	}
	if err == nil {
		err = file.Sync()
	}
	created, _ := file.Stat()
	closeErr := file.Close()
	if err == nil {
		err = closeErr
	}
	if err != nil {
		if current, statErr := root.Lstat("update-token"); statErr == nil && created != nil && os.SameFile(created, current) {
			_ = root.Remove("update-token")
		}
		return "", err
	}
	return path, nil
}

func desktopUpdateToken() (string, error) {
	if configured, set := os.LookupEnv("BEANS_UPDATE_TOKEN_FILE"); set {
		if configured == "" {
			return "", errors.New("Desktop update installation is disabled by BEANS_UPDATE_TOKEN_FILE")
		}
		return "", errors.New("Desktop update installation requires app-owned control; the configured operator token is not read")
	}
	root, err := openDesktopTokenDirectory()
	if err != nil {
		return "", err
	}
	defer root.Close()
	data, err := readDesktopControlFile(root, "update-token", true)
	if err != nil {
		return "", err
	}
	token := strings.TrimSpace(string(data))
	if len(token) < 32 {
		return "", errors.New("The update-control token is invalid")
	}
	return token, nil
}

// A separate, non-reconnecting socket keeps prepare/status/cancel on the same
// Runner even if Settings changes the app's CLI port during the quit check.
type desktopUpdateLease struct {
	client           *cliClient
	token            string
	port             int
	generation       int
	clientGeneration int
	pid              int
	done             <-chan struct{}
	attempted        bool
	expires          time.Time
	pages            []desktopUpdatePage
}

func newDesktopUpdateLease(ctx context.Context) (*desktopUpdateLease, error) {
	if app.launcher == nil || app.cli == nil {
		return nil, errors.New("Wait for the local CLI before installing the update")
	}
	selectedPort, err := prefs.cliPort()
	if err != nil {
		return nil, err
	}
	token, err := desktopUpdateToken()
	if err != nil {
		return nil, err
	}
	l := app.launcher
	l.mu.Lock()
	if l.stopped || !l.ready || l.port != selectedPort || l.process == nil || l.process.Process == nil || l.processDone == nil {
		l.mu.Unlock()
		return nil, errors.New("Installation needs the app's own drain-capable CLI; quit the external CLI and reopen the app")
	}
	lease := &desktopUpdateLease{token: token, port: l.port, generation: l.generation,
		pid: l.process.Process.Pid, done: l.processDone}
	l.mu.Unlock()
	app.cli.mu.Lock()
	lease.clientGeneration = app.cli.generation
	connected := app.cli.state == "connected"
	app.cli.mu.Unlock()
	if !connected {
		return nil, errNotRunning
	}
	conn, _, err := websocket.Dial(ctx, "ws://127.0.0.1:"+strconv.Itoa(lease.port)+"/ws", nil)
	if err != nil {
		return nil, err
	}
	conn.SetReadLimit(256 * 1024 * 1024)
	lease.client = newCLIClient()
	lease.client.conn, lease.client.state = conn, "connected"
	go lease.client.read(lease.client.generation, conn)
	if err = lease.current(); err != nil {
		lease.client.disconnect()
		return nil, err
	}
	return lease, nil
}

func (lease *desktopUpdateLease) current() error {
	port, err := prefs.cliPort()
	if err != nil || port != lease.port {
		return errors.New("The selected CLI changed; quit again to check the update")
	}
	l := app.launcher
	l.mu.Lock()
	current := !l.stopped && l.ready && l.generation == lease.generation && l.port == lease.port &&
		l.process != nil && l.process.Process != nil && l.process.Process.Pid == lease.pid
	l.mu.Unlock()
	app.cli.mu.Lock()
	current = current && app.cli.state == "connected" && app.cli.generation == lease.clientGeneration
	app.cli.mu.Unlock()
	if !current {
		return errors.New("The local CLI changed; quit again to check the update")
	}
	return nil
}

func (lease *desktopUpdateLease) prepare(ctx context.Context) error {
	if err := lease.current(); err != nil {
		return err
	}
	before, err := lease.client.request(ctx, "update.status", nil)
	if err != nil {
		return err
	}
	var existing struct {
		PID      int  `json:"pid"`
		Control  bool `json:"control"`
		Prepared bool `json:"prepared"`
	}
	if err = json.Unmarshal(before, &existing); err != nil {
		return err
	}
	if existing.PID != lease.pid || !existing.Control || existing.Prepared {
		return errors.New("The app's Runner needs available update control before installation")
	}
	params, err := json.Marshal(map[string]any{"token": lease.token, "ttl": 1800})
	if err != nil {
		return err
	}
	lease.attempted = true
	data, err := lease.client.request(ctx, "update.prepare", params)
	if err != nil {
		return err
	}
	var status struct {
		Ready     bool   `json:"ready"`
		Prepared  bool   `json:"prepared"`
		PID       int    `json:"pid"`
		LeaseID   string `json:"lease_id"`
		ExpiresIn int    `json:"expires_in"`
	}
	if err = json.Unmarshal(data, &status); err != nil {
		return err
	}
	if !status.Ready || !status.Prepared || status.PID != lease.pid || status.LeaseID == "" || status.ExpiresIn < 600 {
		return errors.New("Finish admitted Runner work before installing the update")
	}
	lease.expires = time.Now().Add(time.Duration(status.ExpiresIn) * time.Second)
	return lease.current()
}

func (lease *desktopUpdateLease) verify(ctx context.Context) error {
	if err := lease.current(); err != nil {
		return err
	}
	data, err := lease.client.request(ctx, "update.status", nil)
	if err != nil {
		return err
	}
	var status struct {
		Ready     bool `json:"ready"`
		Prepared  bool `json:"prepared"`
		PID       int  `json:"pid"`
		ExpiresIn int  `json:"expires_in"`
	}
	if err = json.Unmarshal(data, &status); err != nil {
		return err
	}
	if !status.Ready || !status.Prepared || status.PID != lease.pid || status.ExpiresIn < 600 ||
		time.Until(lease.expires) < 10*time.Minute {
		return errors.New("The Runner update lease ended; quit again to check the update")
	}
	return lease.current()
}

type desktopUpdatePage struct {
	window *mygo.Window
	inert  bool
}

func (lease *desktopUpdateLease) guardPages(ctx context.Context) error {
	for _, win := range mygo.Windows() {
		value, err := win.EvalContext(ctx, `(() => {
			if (typeof window.beansUpdateHasDraft !== "function" || window.beansUpdateHasDraft() || document.querySelector(".sheet-frame")) return null;
			const inert = document.documentElement.inert;
			document.documentElement.inert = true;
			return inert;
		})()`)
		previous, ok := value.(bool)
		if err != nil || !ok {
			return errors.New("Save or send drafts and close editing sheets before quitting to install the update")
		}
		lease.pages = append(lease.pages, desktopUpdatePage{win, previous})
	}
	return nil
}

func (lease *desktopUpdateLease) restorePages() {
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	for _, page := range lease.pages {
		for _, win := range mygo.Windows() {
			if win == page.window {
				_, _ = win.EvalContext(ctx, `document.documentElement.inert = `+strconv.FormatBool(page.inert))
			}
		}
	}
}

func (lease *desktopUpdateLease) cancel() {
	if lease == nil {
		return
	}
	defer lease.client.disconnect()
	defer lease.restorePages()
	if !lease.attempted {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
	defer cancel()
	params, err := json.Marshal(map[string]any{"token": lease.token})
	if err == nil {
		if _, err = lease.client.request(ctx, "update.cancel", params); err != nil {
			log.Print("Beans updates: could not release the Runner lease; it will expire")
		}
	}
}

func cancelDesktopUpdateQuit(lease *desktopUpdateLease) {
	lease.cancel()
	mygo.RunOnMain(func() {
		u := &desktopUpdater
		u.mu.Lock()
		u.checkingQuit = false
		u.mu.Unlock()
	})
}

// Native dialogs use the same live L() lookup as the page and menu bar.
func desktopUpdateWord(key string, args ...string) string {
	params, err := json.Marshal(struct {
		Key  string   `json:"key"`
		Args []string `json:"args"`
	}{key, args})
	if err != nil {
		return key
	}
	ctx, cancel := context.WithTimeout(context.Background(), 2*time.Second)
	defer cancel()
	for _, win := range mygo.Windows() {
		value, err := win.EvalContext(ctx, `typeof window.beansUpdateWord === "function" ? window.beansUpdateWord(`+string(params)+`) : null`)
		if text, ok := value.(string); err == nil && ok {
			return text
		}
	}
	for _, arg := range args {
		key = strings.Replace(key, "%@", arg, 1)
	}
	return key
}

func useUpdater() {
	// MyGo v0.1.22's httpGet uses the standard DefaultClient. Pin only
	// update-tagged requests; preserve the original client for every other caller.
	client := *http.DefaultClient
	base := client.Transport
	if base == nil {
		base = http.DefaultTransport
	}
	client.Transport = verifiedUpdateTransport{base: base}
	http.DefaultClient = &client
	dir, err := mygo.App.Path(mygo.PathUserData)
	if err != nil {
		log.Printf("Beans updates: %v", err)
		return
	}
	u := &desktopUpdater
	u.file = filepath.Join(dir, "updater.json")
	data, err := os.ReadFile(u.file)
	switch {
	case errors.Is(err, os.ErrNotExist):
		on := true
		u.prefs.AutomaticChecks = &on
		u.prefs.AutomaticDownloads = true
	case err == nil:
		if err = json.Unmarshal(data, &u.prefs); err != nil {
			off := false
			u.prefs = updatePreferences{AutomaticChecks: &off}
			log.Print("Beans updates: invalid saved preferences; automatic updates remain off")
		}
	default:
		off := false
		u.prefs.AutomaticChecks = &off
		log.Print("Beans updates: saved preferences unavailable; automatic updates remain off")
	}
	mygo.App.WhenReady(func() {
		if !mygo.Updater.Enabled() || isDevelopment() {
			return
		}
		go func() {
			timer := time.NewTicker(15 * time.Minute)
			defer timer.Stop()
			for {
				u.mu.Lock()
				due := (u.prefs.AutomaticChecks == nil || *u.prefs.AutomaticChecks) &&
					time.Since(u.prefs.LastCheck) >= 15*time.Minute && u.pending == nil
				u.mu.Unlock()
				if due {
					u.check(false)
				}
				<-timer.C
			}
		}()
	})
}

func (Host) UpdaterState() UpdaterState {
	u := &desktopUpdater
	u.mu.Lock()
	defer u.mu.Unlock()
	state := UpdaterState{Version: mygo.App.Version()}
	if mygo.Updater.Enabled() && !isDevelopment() {
		state.AutomaticChecks = u.prefs.AutomaticChecks == nil || *u.prefs.AutomaticChecks
		state.AutomaticDownloads = u.prefs.AutomaticDownloads
	}
	if !u.prefs.LastCheck.IsZero() {
		state.LastCheck = u.prefs.LastCheck.Unix()
	}
	return state
}

func (Host) CheckForUpdates() { go desktopUpdater.check(true) }

func (Host) SetAutomaticUpdates(checks, downloads bool) UpdaterState {
	u := &desktopUpdater
	u.mu.Lock()
	u.prefs.AutomaticChecks = &checks
	u.prefs.AutomaticDownloads = downloads
	if !downloads && !u.pendingManual {
		u.pending = nil
	}
	u.saveLocked()
	u.mu.Unlock()
	state := Host{}.UpdaterState()
	UpdaterChanged.Broadcast(state)
	return state
}

func (u *beansUpdater) saveLocked() {
	if u.file == "" {
		return
	}
	data, err := json.Marshal(u.prefs)
	if err == nil {
		err = os.MkdirAll(filepath.Dir(u.file), 0o700)
	}
	if err == nil {
		err = os.WriteFile(u.file+".tmp", data, 0o600)
	}
	if err == nil {
		err = os.Rename(u.file+".tmp", u.file)
	}
	if err != nil {
		log.Printf("Beans updates: saving preferences: %v", err)
	}
}

func (u *beansUpdater) check(user bool) {
	if !mygo.Updater.Enabled() || isDevelopment() {
		return
	}
	u.mu.Lock()
	if u.checking {
		u.mu.Unlock()
		return
	}
	u.checking = true
	u.mu.Unlock()
	defer func() { u.mu.Lock(); u.checking = false; u.mu.Unlock() }()
	ctx, cancel := context.WithTimeout(context.Background(), 90*time.Second)
	defer cancel()
	ready, err := readyDesktopRelease(ctx)
	var up *mygo.Update
	if err == nil {
		ctx = context.WithValue(ctx, verifiedUpdateContextKey{}, ready)
		up, err = mygo.Updater.Check(ctx)
		if err == nil && up != nil && up.Version != ready.Version {
			err = errors.New("MyGo metadata does not match the signed Beans release")
		}
	}
	if err != nil {
		if user {
			_, _ = mygo.Dialog.Message(mygo.MessageOptions{Type: mygo.MessageError, Message: desktopUpdateWord("The Beans update is not ready"), Detail: err.Error()})
		}
		return
	}
	u.mu.Lock()
	u.prefs.LastCheck = time.Now()
	automatic, skipped := u.prefs.AutomaticDownloads, u.prefs.SkippedVersion
	u.saveLocked()
	u.mu.Unlock()
	UpdaterChanged.Broadcast(Host{}.UpdaterState())
	if up == nil {
		if user {
			_, _ = mygo.Dialog.Message(mygo.MessageOptions{Message: desktopUpdateWord("You're up to date")})
		}
		return
	}
	if !user && skipped == up.Version {
		return
	}
	accepted := automatic && !user
	if user || !automatic {
		result, err := mygo.Dialog.Message(mygo.MessageOptions{
			Message: desktopUpdateWord("Beans %@ is available", up.Version),
			Detail:  up.Notes + "\n\n" + desktopUpdateWord("Install on normal quit after bot work finishes and drafts are saved. The app will not restart automatically."),
			Buttons: []string{desktopUpdateWord("Install on Quit"), desktopUpdateWord("Later"), desktopUpdateWord("Skip This Version")}, CancelButton: 1,
			CheckboxLabel: desktopUpdateWord("Download and install updates automatically"), CheckboxChecked: automatic,
		})
		if err != nil {
			return
		}
		u.mu.Lock()
		u.prefs.AutomaticDownloads = result.CheckboxChecked
		if result.Button == 2 {
			u.prefs.SkippedVersion = up.Version
		}
		if result.Button == 0 {
			u.prefs.SkippedVersion = ""
		}
		u.saveLocked()
		u.mu.Unlock()
		UpdaterChanged.Broadcast(Host{}.UpdaterState())
		accepted = result.Button == 0
	}
	if accepted {
		u.mu.Lock()
		// Recheck an opt-out made while the network request was in flight.
		if !u.checkingQuit && !u.quitApproved && !u.installationQuitApproved &&
			(user || !automatic || (u.prefs.AutomaticDownloads && (u.prefs.AutomaticChecks == nil || *u.prefs.AutomaticChecks))) {
			u.pending, u.pendingProtocol, u.pendingManual = up, ready.Protocol, user || !automatic
			u.pendingRelease = ready
		}
		u.mu.Unlock()
	}
}

// Called before MyGo closes any page. Pending updates never discard page state
// or stop active work to make a quit succeed. A canceled quit keeps all windows.
func updaterBeforeQuit(event *mygo.QuitEvent) bool {
	u := &desktopUpdater
	u.mu.Lock()
	// A preference change cannot bypass an in-flight guarded quit.
	if u.checkingQuit || u.installationQuitApproved {
		event.PreventDefault()
		u.mu.Unlock()
		return false
	}
	if u.quitApproved {
		lease, pending, relay := u.lease, u.pending, u.relayURL
		u.quitApproved = false
		u.checkingQuit = true
		u.mu.Unlock()
		ctx, cancel := context.WithTimeout(context.Background(), 5*time.Second)
		err := errors.New("Update installation was canceled")
		if pending != nil && lease != nil {
			var selected string
			selected, err = desktopUpdateIdle(ctx, lease.client)
			if err == nil && selected != relay {
				err = errors.New("The selected relay changed before quit")
			}
			if err == nil {
				err = lease.verify(ctx)
			}
		}
		cancel()
		u.mu.Lock()
		if err == nil && u.pending == pending && u.lease == lease {
			u.checkingQuit = false
			u.installationQuitApproved = true
			u.mu.Unlock()
			return true
		}
		u.lease = nil
		u.installationQuitApproved = false
		u.checkingQuit = true
		event.PreventDefault()
		u.mu.Unlock()
		go cancelDesktopUpdateQuit(lease)
		return false
	}
	if u.pending == nil {
		u.mu.Unlock()
		return true
	}
	u.installationQuitApproved = false
	event.PreventDefault()
	u.checkingQuit = true
	pending, selected, required := u.pending, u.pendingRelease, u.pendingProtocol
	u.mu.Unlock()
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		lease, err := newDesktopUpdateLease(ctx)
		var relay string
		if err == nil {
			relay, err = desktopUpdateIdle(ctx, lease.client)
		}
		if err == nil {
			err = lease.guardPages(ctx)
		}
		if err == nil {
			err = verifyUpdateRelay(ctx, relay, required)
		}
		if err == nil {
			err = lease.prepare(ctx)
		}
		if err == nil {
			var current string
			current, err = desktopUpdateIdle(ctx, lease.client)
			if err == nil && current != relay {
				err = errors.New("The selected relay changed; quit again to check the update")
			}
		}
		if err == nil {
			err = lease.verify(ctx)
		}
		mygo.RunOnMain(func() {
			u.mu.Lock()
			if err == nil && (u.pending != pending || u.pendingRelease != selected) {
				err = errors.New("Update installation was canceled")
			}
			if err == nil {
				u.checkingQuit = false
				u.quitApproved = true
				u.relayURL = relay
				u.lease = lease
			}
			u.mu.Unlock()
			if err == nil {
				mygo.App.Quit()
			}
		})
		if err != nil {
			cancelDesktopUpdateQuit(lease)
			_, _ = mygo.Dialog.Message(mygo.MessageOptions{Message: desktopUpdateWord("The update will wait"), Detail: err.Error()})
			return
		}
		// MyGo has no quit-canceled event. If a window or a later listener vetoes
		// Quit, OnQuit never starts. Release admission and reset the app flag.
		time.AfterFunc(time.Second, func() {
			mygo.RunOnMain(func() {
				u.mu.Lock()
				canceled := !u.finishing && u.lease == lease
				if canceled {
					u.lease = nil
					u.quitApproved, u.installationQuitApproved = false, false
					u.checkingQuit = true
					app.quitting = false
				}
				u.mu.Unlock()
				if canceled {
					go cancelDesktopUpdateQuit(lease)
				}
			})
		})
	}()
	return false
}

func desktopUpdateIdle(ctx context.Context, client *cliClient) (string, error) {
	if client == nil || client.currentState() != "connected" {
		return "", errors.New("Wait for the local CLI to reconnect before installing an update")
	}
	data, err := client.request(ctx, "bootstrap", nil)
	if err != nil {
		return "", err
	}
	var snapshot struct {
		RelayURL       string           `json:"relay_url"`
		RunningChatIDs *[]string        `json:"running_chat_ids"`
		RunningTurns   []jsontext.Value `json:"running_turns"`
		Chats          []struct {
			Messages []struct {
				Body struct {
					Run *struct {
						State     string `json:"state"`
						SessionID string `json:"session_id"`
					} `json:"run"`
				} `json:"body"`
			} `json:"messages"`
		} `json:"chats"`
	}
	if err = json.Unmarshal(data, &snapshot); err != nil {
		return "", err
	}
	if snapshot.RunningChatIDs == nil || len(*snapshot.RunningChatIDs) != 0 || len(snapshot.RunningTurns) != 0 {
		return "", errors.New("Finish bot work before quitting to install the update")
	}
	for _, chat := range snapshot.Chats {
		for _, message := range chat.Messages {
			run := message.Body.Run
			if run != nil && run.SessionID != "" && (run.State == "running" || run.State == "waiting") {
				return "", errors.New("Finish running commands before quitting to install the update")
			}
		}
	}
	for _, win := range mygo.Windows() {
		if win != app.main {
			return "", errors.New("Close settings and onboarding windows before installing the update")
		}
		value, err := win.EvalContext(ctx, `typeof window.beansUpdateHasDraft === "function" && !window.beansUpdateHasDraft() && !document.querySelector(".sheet-frame")`)
		if err != nil || value != true {
			return "", errors.New("Save or send drafts and close editing sheets before quitting to install the update")
		}
	}
	return snapshot.RelayURL, nil
}

// app.willTerminate stops the approved idle child first. Wait for its actual
// exit (including Windows resource locks), then install while the instance lock
// remains held. The running app itself is renamed aside by MyGo on Windows.
func finishUpdaterOnQuit() {
	u := &desktopUpdater
	u.mu.Lock()
	u.finishing = true
	up, selected, required, relay, lease := u.pending, u.pendingRelease, u.pendingProtocol, u.relayURL, u.lease
	approved := u.installationQuitApproved
	u.mu.Unlock()
	if lease != nil {
		defer lease.cancel()
	}
	if !approved || up == nil || selected == nil || lease == nil {
		return
	}
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Minute)
	defer cancel()
	// Never swap resources while a child is still using them.
	select {
	case <-lease.done:
	case <-ctx.Done():
		log.Print("Beans update deferred: the local CLI did not exit")
		return
	}
	ready, err := readyDesktopRelease(ctx)
	if err == nil && (ready.Version != up.Version || ready.Protocol != required || ready.metadataHash != selected.metadataHash) {
		err = errors.New("the selected Beans update is no longer the ready release")
	}
	if err == nil {
		err = verifyUpdateRelay(ctx, relay, required)
	}
	if err == nil {
		ctx = context.WithValue(ctx, verifiedUpdateContextKey{}, ready)
		err = up.Install(ctx, nil)
	}
	if err != nil {
		log.Printf("Beans update deferred: %v", err)
	}
}

const beansReleasesURL = "https://github.com/bloodf/beans/releases/download/"

var stableUpdateVersion = regexp.MustCompile(`^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)$`)
var updateAssetName = regexp.MustCompile(`^[A-Za-z0-9][A-Za-z0-9._ -]*$`)
var updateRevision = regexp.MustCompile(`^[0-9a-f]{40}$`)
var updateHash = regexp.MustCompile(`^[0-9a-f]{64}$`)
var updateHTTPClient = &http.Client{Timeout: 30 * time.Second}

type readyUpdateArtifact struct {
	Name      string `json:"name"`
	SHA256    string `json:"sha256"`
	Size      int64  `json:"size"`
	Component string `json:"component"`
	Platform  string `json:"platform"`
	Version   string `json:"version"`
}
type readyUpdateManifest struct {
	Schema    int                   `json:"schema"`
	Version   string                `json:"version"`
	Revision  string                `json:"revision"`
	Protocol  int                   `json:"protocol"`
	Artifacts []readyUpdateArtifact `json:"artifacts"`
}

func updateGET(ctx context.Context, address string, limit int64) ([]byte, error) {
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, address, nil)
	if err != nil {
		return nil, err
	}
	return updateGETRequest(req, limit)
}

func updateGETRequest(req *http.Request, limit int64) ([]byte, error) {
	req.Header.Set("User-Agent", "Beans-Updater")
	// Public GET only: no account/provider credentials, cookies, or GitHub token.
	response, err := updateHTTPClient.Do(req)
	if err != nil {
		return nil, errors.New("update endpoint unavailable")
	}
	defer response.Body.Close()
	if response.StatusCode != http.StatusOK {
		return nil, errors.New("the Beans release is not ready")
	}
	data, err := io.ReadAll(io.LimitReader(response.Body, limit+1))
	if err != nil {
		return nil, errors.New("could not read update metadata")
	}
	if int64(len(data)) > limit {
		return nil, errors.New("update metadata exceeds its size limit")
	}
	return data, nil
}

func readyDesktopRelease(ctx context.Context) (*verifiedDesktopRelease, error) {
	data, err := updateGET(ctx, "https://api.github.com/repos/bloodf/beans/releases?per_page=100", 1<<20)
	if err != nil {
		return nil, err
	}
	var releases []struct {
		Tag        string `json:"tag_name"`
		Draft      bool   `json:"draft"`
		Prerelease bool   `json:"prerelease"`
	}
	if err = json.Unmarshal(data, &releases); err != nil {
		return nil, err
	}
	tag := ""
	for _, release := range releases {
		if !release.Draft && !release.Prerelease && strings.HasPrefix(release.Tag, "beans-v") {
			tag = release.Tag
			break
		}
	}
	version := strings.TrimPrefix(tag, "beans-v")
	if !stableUpdateVersion.MatchString(version) {
		return nil, errors.New("no valid stable Beans release")
	}
	base := beansReleasesURL + tag + "/"
	body, err := updateGET(ctx, base+"beans-update.json", 1<<20)
	if err != nil {
		return nil, err
	}
	signature, err := updateGET(ctx, base+"beans-update.json.sig", 256)
	if err != nil {
		return nil, err
	}
	dir, err := mygo.App.Path(mygo.PathResources)
	if err != nil {
		return nil, err
	}
	keyText, err := os.ReadFile(filepath.Join(dir, "public-key.txt"))
	if err != nil {
		return nil, errors.New("Beans public update key is unavailable")
	}
	key, err := base64.StdEncoding.DecodeString(strings.TrimSpace(string(keyText)))
	if err != nil || len(key) != ed25519.PublicKeySize {
		return nil, errors.New("invalid Beans public update key")
	}
	sig, err := base64.StdEncoding.DecodeString(strings.TrimSpace(string(signature)))
	if err != nil || len(sig) != ed25519.SignatureSize || !ed25519.Verify(key, body, sig) {
		return nil, errors.New("invalid Beans release signature")
	}
	var manifest readyUpdateManifest
	if err = json.Unmarshal(body, &manifest); err != nil {
		return nil, err
	}
	if manifest.Schema != 1 || manifest.Version != version || manifest.Protocol < 5 || !updateRevision.MatchString(manifest.Revision) {
		return nil, errors.New("invalid Beans release manifest")
	}
	seen := make(map[string]readyUpdateArtifact, len(manifest.Artifacts))
	for _, artifact := range manifest.Artifacts {
		_, duplicate := seen[artifact.Name]
		if duplicate || !updateAssetName.MatchString(artifact.Name) || strings.Contains(artifact.Name, "..") ||
			!updateHash.MatchString(artifact.SHA256) || artifact.Size <= 0 || artifact.Size > 1<<30 ||
			artifact.Component == "" || artifact.Platform == "" || !stableUpdateVersion.MatchString(artifact.Version) {
			return nil, errors.New("invalid Beans release artifact")
		}
		seen[artifact.Name] = artifact
	}
	target := runtime.GOOS + "-" + runtime.GOARCH
	metadata, ok := seen["update-"+target+".json"]
	archive, archiveOK := seen["beans-"+version+"-"+target+".tar.gz"]
	if !ok || !archiveOK || metadata.Version != version || archive.Version != version {
		return nil, errors.New("the Beans desktop release is incomplete")
	}
	data, err = updateGET(ctx, base+metadata.Name, 1<<20)
	if err != nil {
		return nil, err
	}
	hash := sha256.Sum256(data)
	if int64(len(data)) != metadata.Size || hex.EncodeToString(hash[:]) != metadata.SHA256 {
		return nil, errors.New("MyGo metadata does not match the signed Beans release")
	}
	var platformManifest struct {
		Version   string           `json:"version"`
		URL       string           `json:"url"`
		Size      int64            `json:"size"`
		Signature string           `json:"signature"`
		Deltas    []jsontext.Value `json:"deltas"`
	}
	if err = json.Unmarshal(data, &platformManifest); err != nil {
		return nil, err
	}
	if platformManifest.Version != version || platformManifest.URL != base+archive.Name ||
		platformManifest.Size != archive.Size || platformManifest.Signature == "" || len(platformManifest.Deltas) != 0 {
		return nil, errors.New("MyGo archive metadata does not match the Beans release")
	}
	// MyGo signs the SHA-256 digest, not the archive bytes. Checking that
	// signature against the signed readiness digest lets MyGo's own streaming
	// verifier enforce the readiness hash without hashing the archive twice.
	digest, err := hex.DecodeString(archive.SHA256)
	if err != nil {
		return nil, err
	}
	archiveSig, err := base64.StdEncoding.DecodeString(platformManifest.Signature)
	if err != nil || !ed25519.Verify(key, digest, archiveSig) {
		return nil, errors.New("MyGo archive signature does not authenticate the signed readiness hash")
	}
	return &verifiedDesktopRelease{
		readyUpdateManifest: manifest, metadataURL: base + metadata.Name,
		metadata: data, metadataHash: metadata.SHA256, archiveURL: base + archive.Name,
	}, nil
}

func verifyUpdateRelay(ctx context.Context, selected string, required int) error {
	u, err := url.Parse(selected)
	if err != nil || u.Host == "" || (u.Scheme != "https" && u.Scheme != "http") || u.User != nil ||
		u.RawQuery != "" || u.ForceQuery || u.Fragment != "" || required < 5 {
		return errors.New("select a Beans relay and upgrade it before installing this client")
	}
	req, err := http.NewRequestWithContext(ctx, http.MethodGet, strings.TrimRight(selected, "/")+"/v1/health", nil)
	if err != nil {
		return errors.New("select a valid Beans relay before installing this client")
	}
	req.Header.Set("Beans-Protocol", "5")
	req.Header.Set("Beans-Format", "beans-v2")
	data, err := updateGETRequest(req, 4096)
	if err != nil {
		return errors.New("the selected relay must be reachable before this protocol update")
	}
	var health struct {
		OK                  bool   `json:"ok"`
		Service             string `json:"service"`
		Format              string `json:"format"`
		Protocol            int    `json:"protocol"`
		MinProtocol         int    `json:"min_protocol"`
		MinRosterProtocol   int    `json:"min_roster_protocol"`
		MemoryConfigVersion int    `json:"memory_config_version"`
	}
	if json.Unmarshal(data, &health) != nil || !health.OK || health.Service != "beans-relay" ||
		health.Format != "beans-v2" || health.Protocol < required || health.MinProtocol < 5 || health.MinProtocol > required ||
		health.MinRosterProtocol < health.MinProtocol || health.MinRosterProtocol > required || health.MemoryConfigVersion != 1 {
		return fmt.Errorf("upgrade the selected relay to Beans v2 protocol %d with enforced compatible floors before installing this client", required)
	}
	return nil
}

type verifiedDesktopRelease struct {
	readyUpdateManifest
	metadataURL  string
	metadata     []byte
	metadataHash string
	archiveURL   string
}

type verifiedUpdateContextKey struct{}
type verifiedUpdateTransport struct{ base http.RoundTripper }

func (t verifiedUpdateTransport) RoundTrip(request *http.Request) (*http.Response, error) {
	ready, pinned := request.Context().Value(verifiedUpdateContextKey{}).(*verifiedDesktopRelease)
	if !pinned {
		return t.base.RoundTrip(request)
	}
	if request.Method != http.MethodGet || request.URL.Scheme != "https" || request.URL.User != nil {
		return nil, errors.New("invalid Beans update request")
	}
	if request.URL.String() == ready.metadataURL {
		// A verified HTTP snapshot, not a second mutable metadata download.
		return &http.Response{
			StatusCode: http.StatusOK, Status: "200 OK", Request: request,
			Header:        http.Header{"Content-Type": []string{"application/json"}},
			ContentLength: int64(len(ready.metadata)), Body: io.NopCloser(bytes.NewReader(ready.metadata)),
		}, nil
	}
	api := request.URL.String() == "https://api.github.com/repos/bloodf/beans/releases?per_page=100&page=1"
	archive := request.URL.String() == ready.archiveURL
	// A GitHub storage URL is allowed only as a redirect of this exact signed
	// archive, never merely because an arbitrary request uses a storage host.
	redirect := false
	if request.Response != nil && request.Response.Request != nil &&
		(request.URL.Host == "release-assets.githubusercontent.com" || request.URL.Host == "objects.githubusercontent.com") {
		origin := request.Response.Request
		for origin.Response != nil && origin.Response.Request != nil {
			origin = origin.Response.Request
		}
		redirect = origin.URL.String() == ready.archiveURL
	}
	if !api && !archive && !redirect {
		return nil, errors.New("refusing an unapproved Beans update URL")
	}
	// DefaultClient may have a cookie jar; public updates never inherit it.
	clean := request.Clone(request.Context())
	clean.Header.Del("Cookie")
	clean.Header.Del("Authorization")
	return t.base.RoundTrip(clean)
}
