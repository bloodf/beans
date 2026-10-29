package main

import (
	"testing"

	"github.com/bloodf/beans/desktop/l10n"
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
