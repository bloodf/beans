package main

import (
	"runtime"
	"strings"

	"github.com/egoist/mygo"
)

// Native shell commands (adapted from Lorca commands.go). The Solid page keeps
// its own command table; these drive the opt-in native chrome and menus.
type nativeCommand struct {
	id          string
	title       func() string
	accelerator string
	role        mygo.MenuRole
	enabled     func() bool
}

var nativeCommandTable = []nativeCommand{
	{id: "newBot", title: func() string { return L("New Bot…") }, accelerator: "CmdOrCtrl+N"},
	{id: "newGroupChat", title: func() string { return L("New Group Chat…") }, accelerator: "CmdOrCtrl+Shift+N"},
	{id: "marketplace", title: func() string { return L("Marketplace…") }, accelerator: "CmdOrCtrl+Shift+M"},
	{id: "pairDevice", title: func() string { return L("Pair a Device…") }, accelerator: "CmdOrCtrl+Shift+P"},
	{id: "settings", title: func() string { return L("Settings…") }, accelerator: "CmdOrCtrl+,"},
	{id: "closeWindow", title: func() string { return L("Close Window") }, accelerator: "CmdOrCtrl+W", role: mygo.RoleClose},
	{id: "quit", title: func() string { return L("Quit %@", appName()) }, accelerator: "CmdOrCtrl+Q", role: mygo.RoleQuit},
	{id: "toggleSidebar", title: func() string { return L("Toggle Sidebar") }, accelerator: "CmdOrCtrl+B"},
	{id: "toggleInspector", title: func() string { return L("Toggle Inspector") }, accelerator: "CmdOrCtrl+Shift+B"},
	{id: "scrollToLatest", title: func() string { return L("Scroll to Latest") }, accelerator: "CmdOrCtrl+J"},
	{id: "fullScreen", title: func() string {
		if native != nil && native.win != nil && native.win.IsFullScreen() {
			return L("Exit Full Screen")
		}
		return L("Enter Full Screen")
	}, accelerator: "F11", role: mygo.RoleToggleFullScreen},
	{id: "help", title: func() string { return L("Beans Help") }, accelerator: "F1"},
	{id: "architecture", title: func() string { return L("Architecture Notes") }},
	{id: "checkForUpdates", title: func() string { return L("Check for Updates…") }},
	{id: "about", title: func() string { return L("About %@", appName()) }},
	{id: "showOnboarding", title: func() string { return L("Show Onboarding") }},
	{id: "openFullClient", title: func() string { return L("Open full client") }},
}

func nativeCommandByID(id string) *nativeCommand {
	for i := range nativeCommandTable {
		if nativeCommandTable[i].id == id {
			return &nativeCommandTable[i]
		}
	}
	return nil
}

func runNativeCommand(id string) {
	switch id {
	case "toggleSidebar":
		native.shell.toggleSidebar()
	case "toggleInspector":
		native.shell.toggleInspector()
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

func shortcutText(accel string) string {
	mod := "Ctrl"
	if runtime.GOOS == "darwin" {
		mod = "⌘"
	}
	out := strings.ReplaceAll(accel, "CmdOrCtrl+", mod+"+")
	out = strings.ReplaceAll(out, "Cmd+", "⌘+")
	return out
}
