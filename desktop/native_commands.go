package main

import (
	"runtime"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

func runNativeCommand(id string) {
	switch id {
	case "toggleSidebar":
		native.toggleSidebar()
	case "toggleInspector":
		native.toggleInspector()
	case "settings":
		native.shell.selectSettings("general")
	case "showOnboarding":
		native.shell.selectOnboarding()
	case "openFullClient":
		app.showMainWindow()
	case "newBot", "newGroupChat", "marketplace", "pairDevice", "help", "architecture", "about", "checkForUpdates":
		app.showMainWindow()
	case "quit":
		mygo.App.Quit()
	}
	if native.win != nil {
		native.win.Invalidate()
	}
}

// nativeShortcuts registers the accelerators advertised on shell chrome tooltips
// (CmdOrCtrl+B / Shift+B / ,). Called each frame from the native window view.
func (n *nativeDesktop) nativeShortcuts(c *ui.Context) {
	if c.Shortcut(ui.Cmd, ui.KeyB) {
		runNativeCommand("toggleSidebar")
	}
	if c.Shortcut(ui.Cmd|ui.Shift, ui.KeyB) {
		runNativeCommand("toggleInspector")
	}
	if c.Shortcut(ui.Cmd, ui.KeyComma) {
		runNativeCommand("settings")
	}
}

func shortcutText(accel string) string {
	mod := "Ctrl"
	if runtime.GOOS == "darwin" {
		mod = "⌘"
	}
	out := strings.ReplaceAll(accel, "CmdOrCtrl+", mod+"+")
	return strings.ReplaceAll(out, "Cmd+", "⌘+")
}
