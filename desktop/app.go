package main

import (
	"context"
	_ "embed"
	"encoding/json/jsontext"
	"errors"
	"github.com/bloodf/beans/desktop/l10n"
	"log"
	"net/url"
	"runtime"
	"sync"
	"time"

	stdjson "encoding/json"
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

//go:embed assets/tray.png
var trayIcon []byte

//go:embed assets/tray-dev.png
var trayIconDev []byte

// CLIState is where the connection to the CLI stands, for the pages' loading and offline states.
type CLIState struct {
	Connection string         `json:"connection"`
	Launcher   LauncherStatus `json:"launcher"`
	Starting   bool           `json:"starting"`
}

func (state CLIState) modelState() model.CLIState {
	return model.CLIState{Connection: state.Connection, Launcher: state.Launcher, Starting: state.Starting}
}
func (state CLIState) MarshalJSON() ([]byte, error) {
	type failure struct {
		Kind   string `json:"kind"`
		Binary string `json:"binary,omitempty"`
		Reason string `json:"reason,omitempty"`
		Code   int    `json:"code"`
		Log    string `json:"log,omitempty"`
	}
	type launcherState struct {
		Kind     string   `json:"kind"`
		External bool     `json:"external,omitempty"`
		Failure  *failure `json:"failure,omitempty"`
	}
	launch := launcherState{Kind: state.Launcher.Kind, External: state.Launcher.External}
	if f := state.Launcher.Failure; f != nil {
		launch.Failure = &failure{f.Kind, f.Binary, f.Reason, f.Code, f.Log}
	}
	return stdjson.Marshal(struct {
		Connection string        `json:"connection"`
		Launcher   launcherState `json:"launcher"`
		Starting   bool          `json:"starting"`
	}{state.Connection, launch, state.Starting})
}

// WindowState tells a page whether its window is where the user looks: in front, shown, not
// minimized. A reply the user watches arrive is neither pushed to their phone nor notified.
type WindowState struct {
	Focused   bool `json:"focused"`
	Visible   bool `json:"visible"`
	Minimized bool `json:"minimized"`
	// FullScreen words the menu's full screen item: Enter or Exit.
	FullScreen bool `json:"fullScreen"`
}

// stateOf is how a window stands, as WindowState answers and WindowStateChanged reports it.
func stateOf(win *mygo.Window) WindowState {
	return WindowState{
		Focused:    win.IsFocused(),
		Visible:    win.IsVisible(),
		Minimized:  win.IsMinimized(),
		FullScreen: win.IsFullScreen(),
	}
}

var (
	// CLIEvents carries every event frame of the CLI, `{ event, data }`, as it came.
	CLIEvents = mygo.NewEvent[jsontext.Value]("cli:event")
	// CLIStateChanged reports the connection and the launcher.
	CLIStateChanged = mygo.NewEvent[CLIState]("cli:state")
	// WindowStateChanged goes to a page when its window gains or loses the user's eye.
	WindowStateChanged = mygo.NewEvent[WindowState]("window:state")
	// OpenChat asks the main window to show a chat, from a clicked notification.
	OpenChat = mygo.NewEvent[string]("open:chat")
)

// store is the app's model: one for every window, on the main thread.
var store *model.Store

// appDelegate is the app's lifecycle: which window a launch opens (onboarding or the main window,
// decided by the identity only the CLI knows), the windows, the CLI's launcher and connection,
// notifications, and the tray icon that keeps the app reachable while its windows are closed.
type appDelegate struct {
	mu                sync.Mutex
	cli               *cliClient
	launcher          *launcher
	session           *model.Session
	sessionGeneration uint64

	main       *mainWindow
	onboarding *onboardingWindow
	settings   *settingsWindow
	// onboardingOverIdentity is onboarding opened again over an identity that still exists.
	// Create, restore, and pair refuse to run there, so closing it is Cancel: the main window it
	// hid comes back as it was.
	onboardingOverIdentity bool

	hasIdentity *bool
	starting    bool
	// stoppedWaiting is a launch done waiting for the CLI's first answer: the CLI failed to start,
	// or answerWait passed. Until then a computer without an identity opens no window, so
	// onboarding never replaces a main window that just appeared.
	stoppedWaiting bool
	tray           *mygo.Tray
	trayWords      [2]string
	quitting       bool
	notifier       *notifier
	// active is the app being the frontmost one, on macOS, where the system says so.
	active bool
}

// answerWait is how long a computer without an identity waits for the CLI's first answer before
// the main window opens on the offline recovery controls. It outlasts the loading state's 2.5
// seconds, which a CLI's first start (a new binary, a new database) can take on its own.
const answerWait = 10 * time.Second

var app = &appDelegate{starting: true}

// cliTransport is the store's way to the CLI: the app's one websocket.
type cliTransport struct{ client *cliClient }

var admittedSnapshot struct {
	sync.Mutex
	data stdjson.RawMessage
}

func (t cliTransport) Request(method string, params any) (stdjson.RawMessage, error) {
	if method == "bootstrap" {
		admittedSnapshot.Lock()
		defer admittedSnapshot.Unlock()
		if admittedSnapshot.data == nil {
			return nil, errors.New("Session bootstrap is not admitted")
		}
		return append(stdjson.RawMessage(nil), admittedSnapshot.data...), nil
	}
	raw, err := stdjson.Marshal(params)
	if err != nil {
		return nil, err
	}
	authority := t.client.captureAuthority()
	result, err := t.client.requestBound(context.Background(), method, raw, &authority)
	return stdjson.RawMessage(result), err
}

func (t cliTransport) Reconnect() { t.client.reconnect() }

func (a *appDelegate) state() CLIState {
	a.mu.Lock()
	starting := a.starting
	a.mu.Unlock()
	return CLIState{Connection: a.cli.currentState(), Launcher: a.launcher.currentStatus(), Starting: starting}
}

func (a *appDelegate) publishState() {
	state := a.state()
	CLIStateChanged.Broadcast(state)
	postMain(func() {
		if state.Connection != "connected" {
			store.CLIStateChanged(state.modelState())
		}
	})
}

func (a *appDelegate) didFinishLaunching() {
	startupTrace("did finish launching")
	l10n.Set(prefs.get().AppLanguage, mygo.App.Locale())
	a.cli = newCLIClient()
	a.launcher = newLauncher()
	store = model.NewStore(cliTransport{a.cli}, postMain, isMock())
	a.notifier = newNotifier()
	store.Subscribe(func(event model.Event) {
		a.storeChanged(event)
		if a.main != nil {
			a.main.storeChanged(event)
		}
		a.notifier.storeChanged(event)
		invalidateWindows()
	})
	if isMock() {
		store.Start()
	}
	a.session = model.NewSession(a.identityChanged, func(name string, data stdjson.RawMessage) {
		if name == "snapshot" || (name == "identity.changed" && !identityPresent(data)) {
			admittedSnapshot.Lock()
			admittedSnapshot.data = append(stdjson.RawMessage(nil), data...)
			admittedSnapshot.Unlock()
			// Fence the Store's asynchronous snapshot completion before admitting this account.
			state := a.state()
			state.Connection = "disconnected"
			store.CLIStateChanged(state.modelState())
			state.Connection = "connected"
			store.CLIStateChanged(state.modelState())
		} else {
			store.HandleEvent(name, data)
		}
		native.event(name, data)
		if name == "snapshot" && identityPresent(data) {
			go remoteUpdates.connected()
		}
		switch name {
		case "app.update.install":
			if a.hasIdentity != nil && *a.hasIdentity {
				go remoteUpdates.install()
			}
		case "app.update.check":
			if a.hasIdentity != nil && *a.hasIdentity {
				go remoteUpdates.checkNow()
			}
		}
		frame, err := stdjson.Marshal(struct {
			Event string             `json:"event"`
			Data  stdjson.RawMessage `json:"data"`
		}{name, data})
		if err == nil {
			CLIEvents.Broadcast(jsontext.Value(frame))
		}
	})
	if isMock() {
		yes := true
		a.hasIdentity = &yes
		a.starting = false
	} else {
		a.cli.onState = func(state string) {
			postMain(func() {
				generation := a.connectionTransition(state)
				if state == "connected" {
					authority := native.authority
					go a.askIdentity(generation, authority)
				}
				if native.win != nil {
					native.win.Invalidate()
				}
				a.publishState()
			})
		}
		a.cli.onEvent = a.cliEvent
		a.cli.onReconnectNeeded = a.launcher.ensureRunning
		a.launcher.onStatus = func(status LauncherStatus) {
			// A child that is starting or restarting owns the next connection attempt.
			if status.Kind == "starting" || status.Kind == "failed" {
				a.cli.disconnect()
			}
			if status.Kind == "failed" {
				a.finishStartup()
				a.stopWaiting()
			}
			a.publishState()
		}
		a.launcher.onReady = a.cli.connect
		a.launcher.ensureRunning()
		// A slow or silent CLI leaves the user with the offline recovery controls: in the main
		// window that is up once the loading state ends, and on a computer without an identity,
		// which opens its window on the CLI's answer, once the app stops waiting for it.
		time.AfterFunc(2500*time.Millisecond, a.finishStartup)
		time.AfterFunc(answerWait, a.stopWaiting)
	}
	a.installTray()
	installMenuBar()
	if a.mainWindowIsDue() {
		a.showMainWindow()
		startupTrace("window shown")
	}
	if nativeEnabled() {
		native.show()
	}
}

// storeChanged keeps what the app shows outside its windows in step with the store: the unread
// badge and the tray's words.
func (a *appDelegate) storeChanged(event model.Event) {
	a.presentPendingTemplateLink()
	switch event.Kind {
	case model.EventIdentityChanged, model.EventSnapshotReplaced, model.EventChatsChanged:
		setBadge(unreadBadge())
	}
}

// unreadBadge is the count on the app's icon: the unread replies of the chats that are not muted.
func unreadBadge() int {
	count := 0
	if store.HasIdentity == nil || *store.HasIdentity {
		// A muted chat keeps its count in the sidebar but leaves the badge alone.
		now := time.Now()
		for _, chat := range store.Chats {
			if !chat.IsMuted(now) {
				count += chat.UnreadCount
			}
		}
	}
	return count
}

func (a *appDelegate) finishStartup() {
	a.mu.Lock()
	changed := a.starting
	a.starting = false
	a.mu.Unlock()
	if changed {
		a.publishState()
	}
}

// stopWaiting gives up on the CLI's first answer: a computer still waiting for it to pick a
// window opens the main window, whose recovery controls say why the CLI is not answering.
func (a *appDelegate) stopWaiting() {
	a.mu.Lock()
	changed := !a.stoppedWaiting
	a.stoppedWaiting = true
	a.mu.Unlock()
	if changed {
		post(a.showMainWindowIfDue)
	}
}

func (a *appDelegate) connectionTransition(state string) uint64 {
	fenceTemplateImport()
	admittedSnapshot.Lock()
	admittedSnapshot.data = nil
	admittedSnapshot.Unlock()
	offline := a.state()
	offline.Connection = "disconnected"
	store.CLIStateChanged(offline.modelState())
	native.store.Fence()
	native.resetDiagnostics()
	native.suspendMemoryForms()
	if native.avatars != nil {
		native.avatars.reset(native.store.Epoch)
		if native.activity != nil {
			native.activity.reset()
		}
	}
	if native.memory != nil {
		_ = native.memory.reset(native.store.Epoch)
	}
	if state == "connected" {
		native.authority = a.cli.captureAuthority()
		a.sessionGeneration = a.session.Connect()
	} else {
		a.session.Disconnect()
	}
	return a.sessionGeneration
}

// askIdentity asks a CLI that just answered whether it holds an identity, which decides between
// onboarding and the main window.
func (a *appDelegate) askIdentity(generation uint64, authority cliAuthority) {
	ctx, cancel := context.WithTimeout(context.Background(), 10*time.Second)
	defer cancel()
	result, err := a.cli.requestBound(ctx, "bootstrap", nil, &authority)
	postMain(func() {
		if err == nil {
			_ = a.session.Bootstrap(generation, stdjson.RawMessage(result))
		}
		if generation == a.sessionGeneration {
			a.finishStartup()
		}
	})
}

func (a *appDelegate) cliEvent(name string, frame []byte) {
	var payload struct {
		Data stdjson.RawMessage `json:"data"`
	}
	if stdjson.Unmarshal(frame, &payload) != nil {
		return
	}
	postMain(func() { a.session.Event(a.sessionGeneration, name, payload.Data) })
}

// identityChanged opens the window the identity calls for. Onboarding closes itself when it
// finishes, so the phrase step is never yanked away by the `identity.changed` that precedes
// the create response.
func (a *appDelegate) identityChanged(has bool) {
	a.mu.Lock()
	same := a.hasIdentity != nil && *a.hasIdentity == has
	a.hasIdentity = &has
	a.mu.Unlock()
	if same {
		return
	}
	patch := PreferencesPatch{HadIdentity: &has}
	if !has {
		patch.UnfinishedOnboarding = new(string)
	}
	saved, err := prefs.update(patch)
	if err != nil {
		log.Printf("identity preferences: %v", err)
	}
	unfinished := saved.UnfinishedOnboarding
	postMain(func() {
		if has {
			if a.onboarding == nil && a.main == nil {
				if unfinished != "" {
					a.resumeOnboarding(unfinished)
				} else {
					a.showMainWindow()
				}
			}
			return
		}
		// The account is gone: a main window hidden behind onboarding goes with it, and
		// onboarding opened over the account becomes the real thing. Onboarding opens before the
		// main window closes: without a tray, the app quits once its last window is gone.
		a.onboardingOverIdentity = false
		setBadge(0)
		if a.onboarding == nil {
			a.presentOnboarding()
		}
		if a.main != nil {
			main := a.main
			a.main = nil
			main.win.Destroy()
		}
	})
}

// mainWindowIsDue is whether the main window is the one to show: with an identity, and before
// the CLI's first answer on a computer that had one, or once the app stops waiting for that
// answer (the offline recovery controls). Onboarding left unfinished over an identity created here
// waits for the answer too, which resumes it.
func (a *appDelegate) mainWindowIsDue() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	unfinished := prefs.get().UnfinishedOnboarding != ""
	if a.hasIdentity != nil {
		return *a.hasIdentity && !unfinished
	}
	return (prefs.get().HadIdentity && !unfinished) || a.stoppedWaiting
}

func (a *appDelegate) showMainWindowIfDue() {
	if a.main == nil && a.onboarding == nil && a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// keepsRunning is whether closing the windows leaves the app running: on macOS, as apps do
// there, and wherever the tray icon brings them back. Bots on this computer run as long as it does.
func (a *appDelegate) keepsRunning() bool {
	return runtime.GOOS == "darwin" || a.tray != nil
}

// newWindow opens a window of native UI. On Windows and Linux its menu bar stays out of sight until
// Alt or F10 takes the keyboard to it, and its shortcuts work all along. On macOS it is at the top
// of the screen.
func (a *appDelegate) newWindow(w *appWindow, options mygo.WindowOptions, view func(c *ui.Context)) *mygo.Window {
	options.AutoHideMenuBar = true
	if nativeEnabled() {
		options.Content = ui.View(w.frame(view))
	}
	win := mygo.NewWindow(options)
	w.win = win
	report := func() { a.windowChanged(w, win); _ = WindowStateChanged.Emit(win, stateOf(win)) }
	win.OnFocus(report)
	win.OnBlur(report)
	win.OnShow(report)
	win.OnHide(report)
	win.OnMinimize(report)
	win.OnRestore(report)
	win.OnEnterFullScreen(report)
	win.OnLeaveFullScreen(report)
	win.Page().OnDOMReady(report)
	// The app's pages stay in the window; a link goes to the browser.
	win.Page().OnWillNavigate(func(e *mygo.NavigateEvent) {
		if external(e.URL) {
			e.PreventDefault()
			go mygo.Shell.OpenExternal(e.URL)
		}
	})
	win.Page().SetWindowOpenHandler(func(req mygo.WindowOpenRequest) *mygo.WindowOptions {
		if external(req.URL) {
			go mygo.Shell.OpenExternal(req.URL)
		}
		return nil
	})
	return win
}

// windowState is what a window's events report: where it stands on screen.
type windowState interface {
	IsFocused() bool
	IsVisible() bool
	IsMinimized() bool
}

// windowChanged follows a window coming forward or going away.
func (a *appDelegate) windowChanged(w *appWindow, win windowState) {
	w.focused = win.IsFocused() && win.IsVisible() && !win.IsMinimized()
	// A window takes the keyboard before AppKit finishes deminiaturizing it, so the focus event
	// still reads it minimized: one with the keyboard is on screen.
	w.onScreen = win.IsVisible() && (!win.IsMinimized() || win.IsFocused())
	a.notifier.watchingChanged()
	w.invalidate()
}

// inFront is the app being the one the user uses: frontmost on macOS, and anywhere, one of its
// windows having the keyboard.
func (a *appDelegate) inFront() bool {
	return a.active || mygo.FocusedWindow() != nil
}

// activeChanged follows the app becoming the frontmost one or leaving it.
func (a *appDelegate) activeChanged(active bool) {
	a.active = active
	a.notifier.watchingChanged()
}

func (a *appDelegate) showMainWindow() {
	if a.main == nil {
		startupTrace("window construction started")
		m := newMainWindow()
		options := mygo.WindowOptions{
			Title:           appName(),
			URL:             "/",
			Width:           1180,
			Height:          760,
			MinWidth:        860,
			MinHeight:       520,
			StateKey:        "main",
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
			// The panes carry the title bar: their headers hold the title, the buttons, and the
			// drag regions, and the window controls sit over them. On Windows and Linux the
			// controls go in a top corner and fill the 52-pixel headers or center in them.
			TitleBarStyle:  mygo.TitleBarHidden,
			TitleBarHeight: 52,
			// The sidebar shows the window's material on macOS, and Mica on Windows 11; elsewhere
			// it draws its own color (Context.Vibrancy).
			Vibrancy: mygo.VibrancySidebar,
		}
		// The traffic lights sit in the sidebar's header: the close button 19 points in and down,
		// centered in the 52-point header.
		if runtime.GOOS == "darwin" {
			options.TrafficLightPosition = &mygo.Point{X: 19, Y: 19}
		}
		win := a.newWindow(&m.appWindow, options, m.view)
		win.OnFocus(func() {
			// Back from the browser, perhaps from a routine's setup: what waits on it tries now.
			store.ResubscribeAwaitingSetup()
		})
		win.OnClose(func(e *mygo.CloseEvent) {
			if err := nativeQuitAdmission(); err != nil {
				e.PreventDefault()
				native.show()
				return
			}
			if a.quitting || !a.keepsRunning() || a.main == nil || win.ID() != a.main.win.ID() {
				return
			}
			// The window goes away and comes back as it was, from the tray or a new launch.
			e.PreventDefault()
			win.Hide()
		})
		win.OnClosed(func() {
			if a.main != nil && a.main.win.ID() == win.ID() {
				a.main = nil
			}
		})
		a.main = m
		m.restoreSelection()
	}
	win := a.main.win
	if win == nil {
		// A main window built without one of its own, as tests build it.
		return
	}
	if win.IsMinimized() {
		win.Restore()
	}
	win.Show()
	win.Focus()
	startupTrace("window controller shown")
}

func (a *appDelegate) presentOnboarding() {
	o := newOnboardingWindow()
	win := a.newWindow(&o.appWindow, mygo.WindowOptions{
		Title:           appName(),
		URL:             "/onboarding",
		Width:           660,
		Height:          560,
		UseContentSize:  true,
		DisableResize:   true,
		DisableMinimize: true,
		DisableMaximize: true,
		BackgroundColor: "light-dark(#ffffff, #262628)",
		// The page fills the window, under the window controls (the close button alone on Windows,
		// as a window that can neither minimize nor maximize has), and its background drags the
		// window.
		TitleBarStyle: mygo.TitleBarHidden,
	}, o.view)
	win.OnClose(func(e *mygo.CloseEvent) {
		if a.quitting || a.onboarding == nil || win.ID() != a.onboarding.win.ID() {
			return
		}
		// Closing onboarding opened again over an identity is Cancel.
		if a.onboardingOverIdentity {
			post(a.endOnboarding)
			return
		}
		// Any other onboarding stays the app's window, which the tray or a new launch brings back.
		if a.keepsRunning() {
			e.PreventDefault()
			win.Hide()
		}
	})
	win.OnClosed(func() {
		if a.onboarding != nil && a.onboarding.win.ID() == win.ID() {
			a.onboarding = nil
		}
	})
	a.onboarding = o
	win.SetMenu(windowMenuBar(windowOther))
	win.Show()
	win.Focus()
}

// endOnboarding closes onboarding and the small settings window, and the main window takes over:
// the same one, as it was, when onboarding hid it. The main window shows first: without a tray,
// the app quits once its last window is gone.
func (a *appDelegate) endOnboarding() {
	if a.onboarding == nil {
		return
	}
	onboarding := a.onboarding
	a.onboarding = nil
	a.onboardingOverIdentity = false
	setUnfinishedOnboarding("")
	a.showMainWindow()
	onboarding.win.Destroy()
	if a.settings != nil {
		settings := a.settings
		a.settings = nil
		settings.win.Destroy()
	}
}

// Where onboarding over an identity created here stopped (UnfinishedOnboarding).
const (
	unfinishedPhrase = "phrase"
	unfinishedSetup  = "setup"
)

func setUnfinishedOnboarding(step string) {
	if prefs.get().UnfinishedOnboarding != step {
		setPrefs(PreferencesPatch{UnfinishedOnboarding: &step})
	}
}

// resumeOnboarding opens onboarding where it stopped over an identity created here, which a quit,
// a crash, or a restart left: the backup phrase again, which the CLI re-derives from the master
// secret, or the first bot. A CLI with no phrase to show (this computer no longer holds the
// identity) leaves nothing to resume, and the main window opens.
func (a *appDelegate) resumeOnboarding(step string) {
	if step != unfinishedPhrase {
		a.presentOnboarding()
		a.onboarding.resume(nil)
		return
	}
	store.BackupPhrase(func(phrase []string, err error) {
		if a.onboarding != nil || a.main != nil {
			return
		}
		if err != nil || len(phrase) == 0 {
			setUnfinishedOnboarding("")
			a.showMainWindow()
			return
		}
		a.presentOnboarding()
		a.onboarding.resume(phrase)
	})
}

// showOnboarding opens onboarding again, from Settings › Advanced. It hides the main window
// rather than closing it, so Cancel brings back the pane, chat, and history there.
func (a *appDelegate) showOnboarding() {
	if a.onboarding != nil {
		a.onboarding.win.Show()
		a.onboarding.win.Focus()
		return
	}
	a.mu.Lock()
	a.onboardingOverIdentity = a.hasIdentity != nil && *a.hasIdentity
	a.mu.Unlock()
	if a.main != nil {
		a.main.win.Hide()
	}
	a.presentOnboarding()
}

// showSettings is Settings from the menu: a mode of the main window, or while onboarding is up,
// where there is no main window, a small window of General and Advanced.
func (a *appDelegate) showSettings() {
	if a.onboarding == nil {
		a.showMainWindow()
		if nativeEnabled() {
			a.main.showSettings("")
		} else {
			_ = MenuCommand.Emit(a.main.win, "settings")
		}
		return
	}
	if a.settings == nil {
		s := newSettingsWindow()
		win := a.newWindow(&s.appWindow, mygo.WindowOptions{
			Title:           L("Settings") + " - " + appName(),
			URL:             "/settings-window",
			Width:           560,
			Height:          480,
			UseContentSize:  true,
			DisableResize:   true,
			DisableMaximize: true,
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
		}, s.view)
		win.OnClosed(func() {
			if a.settings != nil && a.settings.win.ID() == win.ID() {
				a.settings = nil
			}
		})
		win.SetMenu(windowMenuBar(windowOther))
		a.settings = s
	}
	a.settings.win.Show()
	a.settings.win.Focus()
}

// reopen is a click on the tray icon or a second launch: onboarding until it finishes, then the
// main window. A launch still waiting for the CLI's answer shows its window when the answer comes.
func (a *appDelegate) reopen() {
	if a.onboarding != nil {
		a.onboarding.win.Show()
		a.onboarding.win.Focus()
	} else if a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// openChat brings the app forward on a chat, from a clicked notification.
func (a *appDelegate) openChat(chatID string) {
	post(func() {
		if a.onboardingOverIdentity {
			a.endOnboarding()
		}
		if a.onboarding != nil {
			a.onboarding.win.Show()
			a.onboarding.win.Focus()
			return
		}
		a.showMainWindow()
		if nativeEnabled() {
			a.main.open(chatID)
		} else {
			_ = OpenChat.Emit(a.main.win, chatID)
		}
	})
}

// installTray puts the app in the notification area on Windows and Linux, where closing its
// windows would otherwise leave nothing to bring it back by, and bots here stop with the app.
func (a *appDelegate) installTray() {
	if runtime.GOOS == "darwin" {
		return
	}
	icon := trayIcon
	if isDevelopment() {
		icon = trayIconDev
	}
	a.trayWords = [2]string{L("Open %@", appName()), L("Quit %@", appName())}
	tray, err := mygo.NewTray(mygo.TrayOptions{
		Icon:    icon,
		ToolTip: appName(),
		Menu:    a.trayMenu(a.trayWords[0], a.trayWords[1]),
	})
	if err != nil {
		return
	}
	// A click on the icon brings the app back, as a click on its Dock icon does; the menu is on
	// the right button on Windows, and all a click shows on Linux.
	tray.OnClick(func() { post(a.reopen) })
	a.tray = tray
}

func (a *appDelegate) trayMenu(open, quit string) *mygo.Menu {
	return mygo.NewMenu([]*mygo.MenuItem{
		{Label: open, Click: func(*mygo.MenuItem, *mygo.Window) { a.reopen() }},
		mygo.Separator(),
		{Label: quit, Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
}

// languageChanged words what the app shows outside its windows in the language now in force.
func (a *appDelegate) languageChanged() {
	words := [2]string{L("Open %@", appName()), L("Quit %@", appName())}
	if a.tray != nil && words != a.trayWords {
		a.trayWords = words
		a.tray.SetMenu(a.trayMenu(words[0], words[1]))
	}
	installMenuBar()
	for _, w := range []*appWindow{onboardingAppWindow(), settingsAppWindow()} {
		if w != nil && w.win != nil {
			w.win.SetMenu(windowMenuBar(windowOther))
		}
	}
	if a.settings != nil {
		a.settings.win.SetTitle(L("Settings") + " - " + appName())
	}
	invalidateWindows()
}

func onboardingAppWindow() *appWindow {
	if app.onboarding == nil {
		return nil
	}
	return &app.onboarding.appWindow
}

func settingsAppWindow() *appWindow {
	if app.settings == nil {
		return nil
	}
	return &app.settings.appWindow
}

func (a *appDelegate) willTerminate() {
	a.quitting = true
	if a.launcher != nil {
		a.launcher.stop()
	}
	if a.cli != nil {
		a.cli.disconnect()
	}
}

// external is a web or mail link, not one of the app's pages or the dev server's.
func external(raw string) bool {
	u, err := url.Parse(raw)
	if err != nil {
		return false
	}
	switch u.Scheme {
	case "mailto":
		return true
	case "http", "https":
		host := u.Hostname()
		return host != "localhost" && host != "127.0.0.1" && host != "mygo.localhost" && host != fileScheme+".localhost"
	}
	return false
}

func identityPresent(data stdjson.RawMessage) bool {
	var payload struct {
		HasIdentity bool `json:"has_identity"`
	}
	return stdjson.Unmarshal(data, &payload) == nil && payload.HasIdentity
}

func (a *appDelegate) isMainWindow(win *mygo.Window) bool {
	return win != nil && a.main != nil && a.main.win != nil && win.ID() == a.main.win.ID()
}
