// Lorca for Windows and Linux: a MyGo app whose Go side launches and talks to the local CLI, and
// whose pages (src/) are the chat UI. It follows the macOS app (macos/) screen for screen.
package main

import (
	"log"

	"github.com/egoist/mygo"
)

func main() {
	// Keep MyGo's instance lock until the on-quit installation has finished.
	// RequestSingleInstanceLock registers its release callback after this one.
	mygo.App.OnQuit(func() {
		app.willTerminate()
		finishUpdaterOnQuit()
	})
	// Starting the app again brings the running one forward, as a click on the Dock icon does.
	if !mygo.App.RequestSingleInstanceLock() {
		return
	}
	prefs.load()
	applyAppearance(prefs.get().Appearance)

	useUpdater()
	mygo.Bind(CLI{}, Host{}, Files{}, Menus{}, &Notices{}, Prefs{})
	if err := mygo.Protocol.HandleFunc(fileScheme, serveFile); err != nil {
		log.Fatal(err)
	}

	mygo.App.OnSecondInstance(func(args []string, workingDir string) { app.reopen() })
	mygo.App.OnActivate(func(hasVisibleWindows bool) {
		if !hasVisibleWindows {
			app.reopen()
		}
	})
	mygo.App.OnWindowAllClosed(func() {
		if !app.keepsRunning() {
			mygo.App.Quit()
		}
	})
	mygo.App.OnBeforeQuit(func(event *mygo.QuitEvent) {
		if updaterBeforeQuit(event) {
			app.quitting = true
		}
	})
	mygo.App.WhenReady(app.didFinishLaunching)
	if err := mygo.App.Run(); err != nil {
		log.Fatal(err)
	}
}
