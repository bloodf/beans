package main

import (
	"context"
	_ "embed"
	"encoding/json/jsontext"
	"net/url"
	"runtime"
	"sync"
	"time"

	stdjson "encoding/json"
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
)

//go:embed assets/tray.png
var trayIcon []byte

//go:embed assets/tray-dev.png
var trayIconDev []byte

// CLIState is where the connection to the CLI stands, for the pages' loading and offline states.
type CLIState struct {
	// Connection is "disconnected", "connecting", or "connected".
	Connection string         `json:"connection"`
	Launcher   LauncherStatus `json:"launcher"`
	// Starting is the first connection still loading: the window shows a spinner, not the
	// offline recovery controls.
	Starting bool `json:"starting"`
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

// appDelegate is the app's lifecycle, after the Mac app's AppDelegate: which window a launch
// opens (onboarding or the main window, decided by the identity only the CLI knows), the
// windows, the CLI's launcher and connection, notifications, and the tray icon that keeps the
// app reachable while its windows are closed.
type appDelegate struct {
	mu                sync.Mutex
	cli               *cliClient
	launcher          *launcher
	session           *model.Session
	sessionGeneration uint64

	main       *mygo.Window
	onboarding *mygo.Window
	settings   *mygo.Window
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
	quitting       bool
}

// answerWait is how long a computer without an identity waits for the CLI's first answer before
// the main window opens on the offline recovery controls. It outlasts the loading state's 2.5
// seconds, which a CLI's first start (a new binary, a new database) can take on its own.
const answerWait = 10 * time.Second

var app = &appDelegate{starting: true}

func (a *appDelegate) state() CLIState {
	a.mu.Lock()
	starting := a.starting
	a.mu.Unlock()
	if isMock() {
		return CLIState{Connection: "connected", Launcher: LauncherStatus{Kind: "running"}}
	}
	return CLIState{Connection: a.cli.currentState(), Launcher: a.launcher.currentStatus(), Starting: starting}
}

func (a *appDelegate) publishState() {
	CLIStateChanged.Broadcast(a.state())
}

func (a *appDelegate) didFinishLaunching() {
	startupTrace("did finish launching")
	a.cli = newCLIClient()
	a.launcher = newLauncher()
	a.session = model.NewSession(a.identityChanged, func(name string, data stdjson.RawMessage) {
		native.event(name, data)
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
	if a.mainWindowIsDue() {
		a.showMainWindow()
		startupTrace("window shown")
	}
	if nativeEnabled() {
		native.show()
	}
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
		mygo.RunOnMain(a.showMainWindowIfDue)
	}
}

func (a *appDelegate) connectionTransition(state string) uint64 {
	native.store.Fence()
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
		a.finishStartup()
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
	_, _ = prefs.update(PreferencesPatch{HadIdentity: &has})
	mygo.RunOnMain(func() {
		if has {
			if a.onboarding == nil && a.main == nil {
				a.showMainWindow()
			}
			return
		}
		// The account is gone: a main window hidden behind onboarding goes with it, and
		// onboarding opened over the account becomes the real thing. Its unread count goes too,
		// since the main window's page that kept it is gone. Onboarding opens before the main
		// window closes: without a tray, the app quits once its last window is gone.
		a.onboardingOverIdentity = false
		Host{}.SetBadge(0)
		if a.onboarding == nil {
			a.presentOnboarding()
		}
		if a.main != nil {
			main := a.main
			a.main = nil
			main.Destroy()
		}
	})
}

// mainWindowIsDue is whether the main window is the one to show: with an identity, and before
// the CLI's first answer on a computer that had one, or once the app stops waiting for that
// answer (the offline recovery controls).
func (a *appDelegate) mainWindowIsDue() bool {
	a.mu.Lock()
	defer a.mu.Unlock()
	if a.hasIdentity != nil {
		return *a.hasIdentity
	}
	return prefs.get().HadIdentity || a.stoppedWaiting
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

func (a *appDelegate) isMainWindow(win *mygo.Window) bool {
	return win != nil && a.main != nil && win.ID() == a.main.ID()
}

func (a *appDelegate) newWindow(options mygo.WindowOptions) *mygo.Window {
	// On Windows and Linux the menu bar stays out of sight until Alt or F10 takes the keyboard to
	// it, and its shortcuts work all along. The Mac's is at the top of the screen.
	options.AutoHideMenuBar = true
	win := mygo.NewWindow(options)
	report := func() { _ = WindowStateChanged.Emit(win, stateOf(win)) }
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

func (a *appDelegate) showMainWindow() {
	if a.main == nil {
		startupTrace("window construction started")
		options := mygo.WindowOptions{
			Title:           appName(),
			URL:             "/",
			Width:           1180,
			Height:          760,
			MinWidth:        860,
			MinHeight:       520,
			StateKey:        "main",
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
			// The page's panes carry the title bar: their headers hold the title, the buttons,
			// and the drag regions, and the window controls sit over them. On Windows and Linux
			// the controls go in a top corner and fill the 52-pixel headers or center in them.
			TitleBarStyle:  mygo.TitleBarHidden,
			TitleBarHeight: 52,
		}
		// The traffic lights sit in the sidebar's header where the Mac app's toolbar puts them:
		// the close button 19 points in and down, centered in the 52-point header.
		if runtime.GOOS == "darwin" {
			options.TrafficLightPosition = &mygo.Point{X: 19, Y: 19}
		}
		win := a.newWindow(options)
		win.OnClose(func(e *mygo.CloseEvent) {
			if a.quitting || !a.keepsRunning() || a.main == nil || win.ID() != a.main.ID() {
				return
			}
			// The window goes away and comes back as it was, from the tray or a new launch.
			e.PreventDefault()
			win.Hide()
		})
		win.OnClosed(func() {
			if a.main != nil && a.main.ID() == win.ID() {
				a.main = nil
			}
		})
		a.main = win
	}
	if a.main.IsMinimized() {
		a.main.Restore()
	}
	a.main.Show()
	a.main.Focus()
	startupTrace("window controller shown")
}

func (a *appDelegate) presentOnboarding() {
	win := a.newWindow(mygo.WindowOptions{
		Title:           appName(),
		URL:             "/onboarding",
		Width:           660,
		Height:          560,
		UseContentSize:  true,
		DisableResize:   true,
		DisableMinimize: true,
		DisableMaximize: true,
		BackgroundColor: "light-dark(#ffffff, #262628)",
		// As the Mac app's: the page fills the window, under the window controls (the close
		// button alone on Windows, as a window that can neither minimize nor maximize has), and
		// its background drags the window.
		TitleBarStyle: mygo.TitleBarHidden,
	})
	win.OnClose(func(e *mygo.CloseEvent) {
		if a.quitting || a.onboarding == nil || win.ID() != a.onboarding.ID() {
			return
		}
		// Closing onboarding opened again over an identity is Cancel.
		if a.onboardingOverIdentity {
			mygo.RunOnMain(a.endOnboarding)
			return
		}
		// Any other onboarding stays the app's window, which the tray or a new launch brings back.
		if a.keepsRunning() {
			e.PreventDefault()
			win.Hide()
		}
	})
	win.OnClosed(func() {
		if a.onboarding != nil && a.onboarding.ID() == win.ID() {
			a.onboarding = nil
		}
	})
	a.onboarding = win
	win.Show()
	win.Focus()
}

// endOnboarding closes onboarding and the small settings window, and the main window takes
// over: the same one, as it was, when onboarding hid it. The main window shows first: without a
// tray, the app quits once its last window is gone.
func (a *appDelegate) endOnboarding() {
	if a.onboarding == nil {
		return
	}
	onboarding := a.onboarding
	a.onboarding = nil
	a.onboardingOverIdentity = false
	a.showMainWindow()
	onboarding.Destroy()
	if a.settings != nil {
		settings := a.settings
		a.settings = nil
		settings.Destroy()
	}
}

// showOnboarding opens onboarding again, from Settings › Advanced. It hides the main window
// rather than closing it, so Cancel brings back the pane, chat, and history there.
func (a *appDelegate) showOnboarding() {
	if a.onboarding != nil {
		a.onboarding.Show()
		a.onboarding.Focus()
		return
	}
	a.mu.Lock()
	a.onboardingOverIdentity = a.hasIdentity != nil && *a.hasIdentity
	a.mu.Unlock()
	if a.main != nil {
		a.main.Hide()
	}
	a.presentOnboarding()
}

// showSettings is Settings from the menu: a mode of the main window, or while onboarding is up,
// where there is no main window, a small window of General and Advanced.
func (a *appDelegate) showSettings() {
	if a.onboarding == nil {
		a.showMainWindow()
		_ = MenuCommand.Emit(a.main, "settings")
		return
	}
	if a.settings == nil {
		win := a.newWindow(mygo.WindowOptions{
			Title:           appName(),
			URL:             "/settings-window",
			Width:           560,
			Height:          480,
			UseContentSize:  true,
			DisableResize:   true,
			DisableMaximize: true,
			BackgroundColor: "light-dark(#ffffff, #1c1c1c)",
		})
		win.OnClosed(func() {
			if a.settings != nil && a.settings.ID() == win.ID() {
				a.settings = nil
			}
		})
		a.settings = win
	}
	a.settings.Show()
	a.settings.Focus()
}

// reopen is a click on the tray icon or a second launch: onboarding until it finishes, then the
// main window. A launch still waiting for the CLI's answer shows its window when the answer comes.
func (a *appDelegate) reopen() {
	if a.onboarding != nil {
		a.onboarding.Show()
		a.onboarding.Focus()
	} else if a.mainWindowIsDue() {
		a.showMainWindow()
	}
}

// menuCommand routes a menu bar item: the app's own commands here, the rest to the page, with
// the main window brought up first for the commands that open it.
func (a *appDelegate) menuCommand(spec MenuItemSpec, win *mygo.Window) {
	switch spec.ID {
	case "settings":
		a.showSettings()
		return
	case "showOnboarding":
		a.showOnboarding()
		return
	case "quit":
		mygo.App.Quit()
		return
	}
	if spec.OpensMain {
		if a.onboarding != nil {
			return
		}
		a.showMainWindow()
		_ = MenuCommand.Emit(a.main, spec.ID)
		return
	}
	target := win
	if target == nil || target.IsDestroyed() {
		target = a.main
	}
	if target != nil {
		_ = MenuCommand.Emit(target, spec.ID)
	}
}

// openChat brings the app forward on a chat, from a clicked notification.
func (a *appDelegate) openChat(chatID string) {
	mygo.RunOnMain(func() {
		if a.onboardingOverIdentity {
			a.endOnboarding()
		}
		if a.onboarding != nil {
			a.onboarding.Show()
			a.onboarding.Focus()
			return
		}
		a.showMainWindow()
		_ = OpenChat.Emit(a.main, chatID)
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
	tray, err := mygo.NewTray(mygo.TrayOptions{
		Icon:    icon,
		ToolTip: appName(),
		Menu:    a.trayMenu("Open "+appName(), "Quit "+appName()),
	})
	if err != nil {
		return
	}
	// A click on the icon brings the app back, as a click on its Dock icon does; the menu is on
	// the right button on Windows, and all a click shows on Linux.
	tray.OnClick(func() { mygo.RunOnMain(a.reopen) })
	a.tray = tray
}

func (a *appDelegate) trayMenu(open, quit string) *mygo.Menu {
	return mygo.NewMenu([]*mygo.MenuItem{
		{Label: open, Click: func(*mygo.MenuItem, *mygo.Window) { a.reopen() }},
		mygo.Separator(),
		{Label: quit, Click: func(*mygo.MenuItem, *mygo.Window) { mygo.App.Quit() }},
	})
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
