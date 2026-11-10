package main

import (
	"strconv"
	"testing"

	"github.com/bloodf/beans/desktop/l10n"
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestNativeSettingsUsesConfiguredPortAndLocalizedLabel(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	withShellPrefs(t)
	old := l10n.Code()
	t.Cleanup(func() { l10n.Set(string(old), "en-US") })
	l10n.Set("en", "en-US")
	if got := L("Port"); got != "Port" {
		t.Fatalf("English port label = %q", got)
	}
	l10n.Set("zh-Hans", "en-US")
	if got := L("Port"); got != "端口" {
		t.Fatalf("Chinese port label = %q", got)
	}
	want := cliPortLocked(Preferences{CLIPort: defaultCLIPort()})
	if got := prefs.get().CLIPort; got != want {
		t.Fatalf("configured CLI port = %d, want %d", got, want)
	}
}

func TestNativeSettingsLanguageButtonUpdatesVisibleStrings(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	withShellPrefs(t)
	oldLocale, oldLanguage := nativeSystemLocale, l10n.Code()
	nativeSystemLocale = func() string { return "en-US" }
	t.Cleanup(func() {
		nativeSystemLocale = oldLocale
		l10n.Set(string(oldLanguage), "en-US")
	})
	language := "en"
	if _, err := prefs.update(PreferencesPatch{AppLanguage: &language}); err != nil {
		t.Fatal(err)
	}
	syncNativeLanguage()
	n := &nativeDesktop{store: model.NewNativeStore(), shell: newNativeShellLayout()}
	tt := ui.NewTester(func(c *ui.Context) { n.settingsGeneral(c, nativeColors(c)) }, 800, 800)
	if err := tt.Click("简体中文"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if prefs.get().AppLanguage != "zh-Hans" || l10n.Code() != l10n.Chinese || !tt.HasText("外观") || tt.HasText("Appearance") {
		t.Fatal("native language button did not replace visible English strings")
	}
	if err := tt.Click("English"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if prefs.get().AppLanguage != "en" || l10n.Code() != l10n.English || !tt.HasText("Appearance") || tt.HasText("外观") {
		t.Fatal("native language button did not restore visible English strings")
	}
}

func TestNativeSettingsRendersAdmittedPort(t *testing.T) {
	unsetDesktopTestEnv(t, "BEANS_PORT")
	withShellPrefs(t)
	oldLanguage := l10n.Code()
	l10n.Set("en", "en-US")
	t.Cleanup(func() { l10n.Set(string(oldLanguage), "en-US") })
	n := &nativeDesktop{store: model.NewNativeStore()}
	tt := ui.NewTester(func(c *ui.Context) { n.settingsAdvanced(c, nativeColors(c)) }, 800, 800)
	if !tt.HasText("Port " + strconv.Itoa(prefs.get().CLIPort)) {
		t.Fatal("advanced settings did not render the admitted port")
	}
	// An incompatible saved port has no admitted endpoint; do not show the
	// build default as if a connection could be made.
	prefs.mu.Lock()
	prefs.value.CLIPort = defaultCLIPort() + 10
	prefs.mu.Unlock()
	tt.Frame()
	if !tt.HasText("Port 0") {
		t.Fatal("advanced settings concealed incompatible saved configuration")
	}
}
