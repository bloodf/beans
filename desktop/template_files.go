package main

import (
	"net/url"
	"os"
	"path/filepath"
	"strings"

	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// The system's Open and Save dialogs for template files, which tests replace. They answer a path;
// the CLI reads and writes the file.
var chooseTemplateSource = openTemplateSource
var chooseTemplateDestination = saveTemplateDestination

// pendingTemplateLink is a shared bot's link the app was opened with, kept until there is an
// account and a CLI to read it: Open in Beans on usebeans.app may be what started the app.
var pendingTemplateLink string

// openTemplateLink takes beans://t/<id>#<key> (beans-dev:// for Beans Dev) from a shared bot's
// page, which the sheet shows as the page's own address.
func (a *appDelegate) openTemplateLink(link string) {
	page, err := url.Parse(link)
	scheme := "beans"
	if isDevelopment() {
		scheme = "beans-dev"
	}
	if err != nil || page.Scheme != scheme || page.Host != "t" || page.User != nil {
		return
	}
	// Preserve the relay query and fragment key for CLI validation.
	page.Scheme, page.Host, page.Path = "https", "usebeans.app", "/t"+page.Path
	pendingTemplateLink = page.String()
	a.presentPendingTemplateLink()
}

func (a *appDelegate) presentPendingTemplateLink() {
	if pendingTemplateLink == "" || store.HasIdentity == nil || !*store.HasIdentity || !(store.IsConnected || store.IsMock) {
		return
	}
	link := pendingTemplateLink
	page, err := url.Parse(link)
	if err != nil {
		return
	}
	if !page.Query().Has("relay") {
		relay, err := url.Parse(store.RelayURL)
		if err != nil || relay.Host == "" || relay.User != nil || (relay.Scheme != "http" && relay.Scheme != "https") {
			return
		}
		query := page.Query()
		query.Set("relay", relay.String())
		page.RawQuery = query.Encode()
		link = page.String()
	}
	a.showMainWindow()
	if a.main == nil {
		return
	}
	if nativeEnabled() {
		pendingTemplateLink = ""
		a.main.presentTemplateImport(link, "", a.main.selectChat)
		return
	}
	// Solid uses the same reviewed native import sheet in a dedicated window.
	// Track it with the other windows so account teardown dismisses its drafts.
	if templateImportWindow == nil {
		w := &appWindow{}
		view := func(c *ui.Context) {}
		win := a.newWindow(w, mygo.WindowOptions{Title: L("New Bot from Template…"), Width: 720, Height: 640, Content: ui.View(w.frame(view))}, view)
		templateImportWindow = w
		win.OnClosed(func() {
			for len(w.sheets) > 0 {
				w.sheets[0].dismiss()
			}
			if templateImportWindow == w {
				templateImportWindow = nil
			}
		})
	}
	w := templateImportWindow
	pendingTemplateLink = ""
	w.presentTemplateImport(link, "", a.openChat)
	w.win.Show()
	w.win.Focus()
}

// copyText puts a shared link on the clipboard; tests keep it off the user's.
var copyText = func(text string) { mygo.Clipboard.WriteText(text) }

func openTemplateSource(parent *mygo.Window, done func(string, error)) {
	options := mygo.OpenDialogOptions{Parent: parent, Filters: []mygo.FileFilter{{Name: L("Bot templates"), Extensions: []string{"beans-template"}}}}
	go func() {
		paths, err := mygo.Dialog.Open(options)
		path := ""
		if len(paths) > 0 {
			path = paths[0]
		}
		post(func() { done(path, err) })
	}()
}

func saveTemplateDestination(parent *mygo.Window, name string, done func(string, error)) {
	options := mygo.SaveDialogOptions{Parent: parent, DefaultPath: templateFilename(name), Filters: []mygo.FileFilter{{Name: L("Bot templates"), Extensions: []string{"beans-template"}}}}
	go func() {
		path, err := mygo.Dialog.Save(options)
		post(func() { done(path, err) })
	}()
}

func templateFilename(name string) string {
	name = strings.Map(func(r rune) rune {
		if r < 32 || strings.ContainsRune(`<>:"/\|?*`, r) {
			return '-'
		}
		return r
	}, name)
	name = strings.Trim(name, " .")
	if name == "" {
		name = "bot"
	}
	stem := strings.ToUpper(strings.SplitN(name, ".", 2)[0])
	if stem == "CON" || stem == "PRN" || stem == "AUX" || stem == "NUL" ||
		(len(stem) == 4 && (strings.HasPrefix(stem, "COM") || strings.HasPrefix(stem, "LPT")) && stem[3] >= '1' && stem[3] <= '9') {
		name = "bot-" + name
	}
	return name + ".beans-template"
}

// Native Save confirms an existing path. If adding the required extension selects a different
// existing path, the sheet explicitly asks before replacing that file too.
func templateDestination(path string) (normalized string, exists, needsConfirmation bool) {
	normalized = path
	if !strings.HasSuffix(path, ".beans-template") {
		if strings.EqualFold(filepath.Ext(path), ".beans-template") {
			normalized = strings.TrimSuffix(path, filepath.Ext(path)) + ".beans-template"
		} else {
			normalized += ".beans-template"
		}
	}
	_, err := os.Lstat(normalized)
	exists = err == nil
	return normalized, exists, exists && normalized != path
}

var templateImportWindow *appWindow

// Drop template drafts whenever the transport authority changes.
func fenceTemplateImport() {
	if w := templateImportWindow; w != nil {
		for len(w.sheets) > 0 {
			w.sheets[0].dismiss()
		}
		templateImportWindow = nil
		w.win.Close()
	}
}
