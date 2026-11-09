package main

import (
	"context"
	"encoding/json/jsontext"
	"fmt"
	"math"
	"net/url"
	"runtime"
	"strconv"
	"strings"
	"sync"
	"unicode/utf16"

	"github.com/egoist/mygo"
)

// CLI is the pages' way to the local CLI, through the app's one websocket.
type CLI struct{}

// State returns where the connection and the launcher stand.
func (CLI) State() CLIState { return app.state() }

// Request sends one call of the CLI's JSON API and returns its result.
func (CLI) Request(ctx context.Context, method string, params jsontext.Value) (jsontext.Value, error) {
	return app.cli.request(ctx, method, params)
}

// Reconnect drops the socket and looks for the CLI again, for Try Again or a new port.
func (CLI) Reconnect() {
	app.cli.reconnect()
}

// HostInfo is what a page knows about the build and the computer it runs on.
type HostInfo struct {
	// Platform is "windows", "linux", or "darwin".
	Platform      string `json:"platform"`
	Name          string `json:"name"`
	Version       string `json:"version"`
	IsDevelopment bool   `json:"isDevelopment"`
	IsMock        bool   `json:"isMock"`
	// CLICommand starts the CLI by hand.
	CLICommand         string `json:"cliCommand"`
	DefaultCLIPort     int    `json:"defaultCLIPort"`
	ProductionRelayURL string `json:"productionRelayURL"`
	// Locale is the system's language, such as "zh-CN".
	Locale         string `json:"locale"`
	UpdatesEnabled bool   `json:"updatesEnabled"`
	CLILogPath     string `json:"cliLogPath"`
	// KeepsRunning is whether closing the windows leaves the app running.
	KeepsRunning bool `json:"keepsRunning"`
}

// Host is the app around the pages: its windows, the badge, the system's shell and clipboard.
type Host struct{}

// Info describes the build and this computer.
func (Host) Info() HostInfo {
	return HostInfo{
		Platform:           platformName(),
		Name:               appName(),
		Version:            mygo.App.Version(),
		IsDevelopment:      isDevelopment(),
		IsMock:             isMock(),
		CLICommand:         cliCommand(),
		DefaultCLIPort:     defaultCLIPort(),
		ProductionRelayURL: productionRelayURL,
		Locale:             mygo.App.Locale(),
		UpdatesEnabled:     updatesEnabled(),
		CLILogPath:         cliLogPath(),
		KeepsRunning:       app.keepsRunning(),
	}
}

// WindowState returns whether the calling page's window is where the user looks.
func (Host) WindowState(ctx context.Context) WindowState {
	win := mygo.CallerWindow(ctx)
	if win == nil {
		return WindowState{}
	}
	return stateOf(win)
}

// ShowMainWindow brings the main window up.
func (Host) ShowMainWindow() { mygo.RunOnMain(app.showMainWindow) }

// FinishOnboarding closes onboarding; the main window takes over.
func (Host) FinishOnboarding() { mygo.RunOnMain(app.endOnboarding) }

// ShowOnboarding opens onboarding again over the main window, from Settings › Advanced.
func (Host) ShowOnboarding() { mygo.RunOnMain(app.showOnboarding) }

// ToggleFullScreen puts the calling page's window in full screen or takes it out, for the command
// palette's Enter Full Screen; the menu's item is the system's own.
func (Host) ToggleFullScreen(ctx context.Context) {
	if win := mygo.CallerWindow(ctx); win != nil {
		win.ToggleFullScreen()
	}
}

// WindowRoom returns how much wider the calling page's window can get on its display, in the
// page's pixels, of which it is `pageWidth` wide (the page's zoom scales them from the window's).
// A maximized or full-screen window has none.
func (Host) WindowRoom(ctx context.Context, pageWidth float64) float64 {
	win, scale := widenable(ctx, pageWidth)
	if win == nil {
		return 0
	}
	bounds := win.Bounds()
	return float64(widened(bounds, workArea(bounds), math.MaxInt32).Width-bounds.Width) / scale
}

// WidenWindow makes the calling page's window `by` of the page's pixels wider, for a side pane the
// user opens: to the right where its display has room, and to the left past that. The right edge
// moves first because the page's frame stays anchored at the left until the page lays out again,
// so a moving left edge would slide the whole page over for a moment.
func (Host) WidenWindow(ctx context.Context, by, pageWidth float64) {
	win, scale := widenable(ctx, pageWidth)
	if win == nil {
		return
	}
	bounds := win.Bounds()
	if next := widened(bounds, workArea(bounds), int(math.Round(by*scale))); next != bounds {
		win.SetBounds(next)
	}
}

// widenable is the calling page's window unless it is maximized or in full screen, with how many
// of the window's pixels make one of the page's, which is `pageWidth` wide.
func widenable(ctx context.Context, pageWidth float64) (*mygo.Window, float64) {
	win := mygo.CallerWindow(ctx)
	if win == nil || pageWidth <= 0 || win.IsMaximized() || win.IsFullScreen() {
		return nil, 0
	}
	width, _ := win.ContentSize()
	if width <= 0 {
		return nil, 0
	}
	return win, float64(width) / pageWidth
}

func workArea(r mygo.Rectangle) mygo.Rectangle {
	return mygo.Screen.DisplayMatching(r).WorkArea
}

// SetBadge shows the chats' unread count where the system has a place for it: the launcher
// entry on Linux, the tray's tooltip elsewhere. Zero clears it.
func (Host) SetBadge(count int) {
	mygo.App.SetBadgeCount(count)
	if app.tray != nil {
		tip := appName()
		if count > 0 {
			tip += " · " + strconv.Itoa(count)
		}
		app.tray.SetToolTip(tip)
	}
}

// SetTrayMenu words the tray icon's menu in the app's language.
func (Host) SetTrayMenu(open, quit string) {
	if app.tray != nil {
		app.tray.SetMenu(app.trayMenu(open, quit))
	}
}

// OpenExternal opens a web link in the browser or a mail link in the mail app. The links come
// from bots, plugins, and the marketplace, so any other scheme is refused: the system would
// start whatever app handles it.
func (Host) OpenExternal(link string) error {
	u, err := url.Parse(link)
	if err != nil || !(u.Scheme == "mailto" || (u.Scheme == "http" || u.Scheme == "https") && u.Host != "") {
		return fmt.Errorf("not a web or mail link: %q", link)
	}
	return mygo.Shell.OpenExternal(link)
}

// CopyText puts text on the clipboard.
func (Host) CopyText(text string) { mygo.Clipboard.WriteText(text) }

// Beep is the system's alert sound, for a command that cannot run.
func (Host) Beep() { mygo.Shell.Beep() }

// NoticeOptions is one system notification: a reply, a failed response, or a question.
type NoticeOptions struct {
	// ID is the message's; a question's notification goes when it is answered.
	ID       string `json:"id"`
	ChatID   string `json:"chatId"`
	Title    string `json:"title"`
	Subtitle string `json:"subtitle,omitempty"`
	Body     string `json:"body"`
}

type posted struct {
	notification *mygo.Notification
	chatID       string
}

// Notices posts system notifications for the chats and takes them back once they are seen. A
// click brings the app forward on the chat.
type Notices struct {
	mu    sync.Mutex
	shown map[string]posted
}

// Supported is whether this build can post notifications.
func (n *Notices) Supported() bool { return mygo.NotificationsSupported() }

// Show posts a notification; one with the same id replaces it.
func (n *Notices) Show(options NoticeOptions) error {
	title, body := options.Title, options.Body
	// Windows shows a notification as a tray balloon, which holds 63 UTF-16 units of title and 255
	// of text, the subtitle's line among them: the words are cut to fit, ending in an ellipsis.
	if runtime.GOOS == "windows" {
		title = fitUTF16(title, 63)
		room := 255
		if options.Subtitle != "" {
			room -= len(utf16.Encode([]rune(options.Subtitle))) + 1
		}
		body = fitUTF16(body, room)
	}
	notification := mygo.NewNotification(mygo.NotificationOptions{
		Title:    title,
		Subtitle: options.Subtitle,
		Body:     body,
	})
	chatID := options.ChatID
	notification.OnClick(func() { app.openChat(chatID) })
	n.Remove(options.ID)
	if err := notification.Show(); err != nil {
		return err
	}
	n.mu.Lock()
	if n.shown == nil {
		n.shown = map[string]posted{}
	}
	n.shown[options.ID] = posted{notification: notification, chatID: chatID}
	n.mu.Unlock()
	return nil
}

// Remove takes back the notification of a message, such as a question answered elsewhere.
func (n *Notices) Remove(id string) {
	n.mu.Lock()
	old, ok := n.shown[id]
	delete(n.shown, id)
	n.mu.Unlock()
	if ok {
		old.notification.Close()
	}
}

// ClearChat takes back what was posted for a chat that is now on screen.
func (n *Notices) ClearChat(chatID string) {
	n.mu.Lock()
	var gone []*mygo.Notification
	for id, entry := range n.shown {
		if entry.chatID == chatID {
			gone = append(gone, entry.notification)
			delete(n.shown, id)
		}
	}
	n.mu.Unlock()
	for _, notification := range gone {
		notification.Close()
	}
}
