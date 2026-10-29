package main

import (
	"strconv"

	"github.com/bloodf/beans/desktop/l10n"
	"github.com/egoist/mygo/ui"
)

// Native settings shell panes (adapted from Lorca settings.go). General and
// Advanced edit preferences already owned by prefs.go; richer panes open the
// full Solid client until later D-slices land.
type nativeSettingsPane struct {
	id    string
	title string
}

func nativeSettingsPanes() []nativeSettingsPane {
	return []nativeSettingsPane{
		{id: "general", title: L("General")},
		{id: "advanced", title: L("Advanced")},
	}
}

func (n *nativeDesktop) settingsSidebar(c *ui.Context) {
	p := nativeColors(c)
	ui.Column(c).Grow(1).Padding(8, 12).Gap(4).Children(func() {
		ui.Text(c, L("Settings")).Bold().FontSize(13).TextColor(p.Label2)
		for _, pane := range nativeSettingsPanes() {
			if ui.Button(c.Key("settings-"+pane.id), pane.title).Selected(n.shell.settingsPane == pane.id).Clicked() {
				n.shell.selectSettings(pane.id)
			}
		}
		if ui.Button(c, L("Open full client")).Clicked() {
			app.showMainWindow()
			app.showSettings()
		}
	})
}

func (n *nativeDesktop) settingsPage(c *ui.Context) {
	p := nativeColors(c)
	ui.Column(c).Grow(1).Padding(20, 24).Gap(16).Children(func() {
		ui.Text(c, settingsPaneTitle(n.shell.settingsPane)).Bold().FontSize(20)
		switch n.shell.settingsPane {
		case "advanced":
			n.settingsAdvanced(c, p)
		default:
			n.settingsGeneral(c, p)
		}
	})
}

func settingsPaneTitle(id string) string {
	for _, pane := range nativeSettingsPanes() {
		if pane.id == id {
			return pane.title
		}
	}
	return L("Settings")
}

func (n *nativeDesktop) settingsGeneral(c *ui.Context, p nativePalette) {
	pref := prefs.get()
	ui.Text(c, L("Appearance")).Bold()
	appearance := pref.Appearance
	for _, opt := range []struct{ id, title string }{
		{"", L("Follow system")},
		{"light", L("Light")},
		{"dark", L("Dark")},
	} {
		if ui.Button(c.Key("appearance-"+opt.id), opt.title).Selected(appearance == opt.id).Clicked() {
			v := opt.id
			prefs.update(PreferencesPatch{Appearance: &v})
			if n.win != nil {
				n.win.Invalidate()
			}
		}
	}
	ui.Text(c, L("Language")).Bold()
	language := pref.AppLanguage
	for _, opt := range []struct{ id, title string }{
		{"", L("Follow system")},
		{string(l10n.English), "English"},
		{string(l10n.Chinese), "简体中文"},
	} {
		if ui.Button(c.Key("lang-"+opt.id), opt.title).Selected(language == opt.id).Clicked() {
			v := opt.id
			prefs.update(PreferencesPatch{AppLanguage: &v})
			syncNativeLanguage()
			if n.win != nil {
				n.win.Invalidate()
			}
		}
	}
	send := pref.SendOnReturn
	if ui.Button(c, L("Send with Return")).Selected(send).Clicked() {
		v := !send
		prefs.update(PreferencesPatch{SendOnReturn: &v})
	}
	stamps := pref.ShowTimestamps
	if ui.Button(c, L("Show timestamps")).Selected(stamps).Clicked() {
		v := !stamps
		prefs.update(PreferencesPatch{ShowTimestamps: &v})
	}
}

func (n *nativeDesktop) settingsAdvanced(c *ui.Context, _ nativePalette) {
	ui.Text(c, cliCommand())
	ui.Text(c, "Port "+strconv.Itoa(defaultCLIPort()))
	if ui.Button(c, L("Show Onboarding")).Clicked() {
		n.shell.selectOnboarding()
	}
	if ui.Button(c, L("Open full client")).Clicked() {
		app.showMainWindow()
		app.showSettings()
	}
}
