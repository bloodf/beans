package main

import (
	"encoding/json"
	"fmt"
	"log"
	"os"
	"path/filepath"
	"strconv"
	"sync"

	"github.com/bloodf/beans/desktop/l10n"
	"github.com/egoist/mygo"
)

// Preferences are this computer's own settings: what the app remembers between launches, and
// the CLI port and relay URL a computer with no CLI answering needs. Nothing here is synced.
type Preferences struct {
	// HadIdentity is whether the CLI's last answer had an identity. Launch opens the main window
	// at once when it did, and otherwise waits for the answer, so a fresh install goes straight
	// to onboarding.
	HadIdentity bool `json:"hadIdentity"`
	// UnfinishedOnboarding is where onboarding for an identity created here stopped short of its
	// last step: "phrase" before the backup phrase was saved, "setup" before the first bot and
	// the provider. A launch with an identity opens onboarding there instead of the main window.
	UnfinishedOnboarding string `json:"unfinishedOnboarding,omitempty"`
	// Selection is the chat the main window showed last ("chat:<id>"), so a relaunch lands back
	// on it rather than on a settings pane.
	Selection      string `json:"selection"`
	ShowsInspector bool   `json:"showsInspector"`
	SendOnReturn   bool   `json:"sendOnReturn"`
	ShowTimestamps bool   `json:"showTimestamps"`
	RelayURL       string `json:"relayURL"`
	// CLIPort must match this build's isolated port, including any explicit BEANS_PORT.
	CLIPort int `json:"cliPort"`
	// AppLanguage is the language the app's own words are in ("en", "zh-Hans"); empty follows
	// the system.
	AppLanguage string `json:"appLanguage"`
	// Appearance is "light" or "dark"; empty follows the system.
	Appearance string `json:"appearance"`
	// The split view's panes, as the user left them.
	SidebarWidth     int  `json:"sidebarWidth"`
	InspectorWidth   int  `json:"inspectorWidth"`
	SidebarCollapsed bool `json:"sidebarCollapsed"`
	// ShowsHiddenChats is the sidebar's Hidden group open; it starts folded. CollapsesOtherChats
	// folds the group of chats in no section. Sections fold on every Device through the roster;
	// these two fold on this computer alone.
	ShowsHiddenChats    bool `json:"showsHiddenChats"`
	CollapsesOtherChats bool `json:"collapsesOtherChats"`
}

// PreferencesPatch changes the preferences it names.
type PreferencesPatch struct {
	HadIdentity          *bool   `json:"hadIdentity,omitempty"`
	UnfinishedOnboarding *string `json:"unfinishedOnboarding,omitempty"`
	Selection            *string `json:"selection,omitempty"`
	ShowsInspector       *bool   `json:"showsInspector,omitempty"`
	SendOnReturn         *bool   `json:"sendOnReturn,omitempty"`
	ShowTimestamps       *bool   `json:"showTimestamps,omitempty"`
	RelayURL             *string `json:"relayURL,omitempty"`
	CLIPort              *int    `json:"cliPort,omitempty"`
	AppLanguage          *string `json:"appLanguage,omitempty"`
	Appearance           *string `json:"appearance,omitempty"`
	SidebarWidth         *int    `json:"sidebarWidth,omitempty"`
	InspectorWidth       *int    `json:"inspectorWidth,omitempty"`
	SidebarCollapsed     *bool   `json:"sidebarCollapsed,omitempty"`
	ShowsHiddenChats     *bool   `json:"showsHiddenChats,omitempty"`
	CollapsesOtherChats  *bool   `json:"collapsesOtherChats,omitempty"`
}

type prefsStore struct {
	mu    sync.Mutex
	path  string
	value Preferences
}

var prefs = &prefsStore{}
var PreferencesChanged = mygo.NewEvent[Preferences]("prefs:changed")

// load reads preferences-v2.json in the app's data directory.
func (s *prefsStore) load() {
	path := ""
	if dir, err := mygo.App.Path(mygo.PathUserData); err == nil {
		path = filepath.Join(dir, "preferences-v2.json")
	}
	s.loadFrom(path)
}

// loadFrom reads the preferences at `path`, which later changes are saved to; none is the defaults.
func (s *prefsStore) loadFrom(path string) {
	s.mu.Lock()
	defer s.mu.Unlock()
	s.value = Preferences{ShowsInspector: true, SendOnReturn: true, ShowTimestamps: true, CLIPort: defaultCLIPort()}
	s.path = path
	if path == "" {
		return
	}
	if data, err := os.ReadFile(path); err == nil {
		if json.Unmarshal(data, &s.value) != nil {
			s.value.CLIPort = 0
		}
	} else if !os.IsNotExist(err) {
		s.value.CLIPort = 0
	}

}

func (s *prefsStore) get() Preferences {
	s.mu.Lock()
	defer s.mu.Unlock()
	value := s.value
	value.CLIPort = cliPortLocked(value)
	return value
}

// Invalid explicit configuration has no usable endpoint; it never selects a fallback.
func (s *prefsStore) cliPort() (int, error) {
	port := s.get().CLIPort
	expected := defaultCLIPort()
	if port != expected {
		return 0, fmt.Errorf("This build connects only to its isolated Beans CLI port %d. Restore that port in Settings › Advanced and check BEANS_PORT.", expected)
	}
	return port, nil
}

func cliPortLocked(value Preferences) int {
	expected := defaultCLIPort()
	if value.CLIPort != expected {
		return 0
	}
	if raw, set := os.LookupEnv("BEANS_PORT"); set {
		port, err := strconv.Atoi(raw)
		if err != nil || port != expected {
			return 0
		}
	}
	return expected
}

func (s *prefsStore) update(patch PreferencesPatch) (Preferences, error) {
	if patch.CLIPort != nil && *patch.CLIPort != defaultCLIPort() {
		return s.get(), fmt.Errorf("This build connects only to its isolated Beans CLI port %d", defaultCLIPort())
	}
	s.mu.Lock()
	candidate := s.value
	if patch.CLIPort != nil {
		candidate.CLIPort = *patch.CLIPort
	}
	if cliPortLocked(candidate) == 0 {
		s.mu.Unlock()
		return s.get(), fmt.Errorf("Restore the isolated Beans CLI port %d and check BEANS_PORT before changing preferences", defaultCLIPort())
	}
	v := &candidate
	if patch.HadIdentity != nil {
		v.HadIdentity = *patch.HadIdentity
	}
	if patch.UnfinishedOnboarding != nil {
		v.UnfinishedOnboarding = *patch.UnfinishedOnboarding
	}
	if patch.Selection != nil {
		v.Selection = *patch.Selection
	}
	if patch.ShowsInspector != nil {
		v.ShowsInspector = *patch.ShowsInspector
	}
	if patch.SendOnReturn != nil {
		v.SendOnReturn = *patch.SendOnReturn
	}
	if patch.ShowTimestamps != nil {
		v.ShowTimestamps = *patch.ShowTimestamps
	}
	if patch.RelayURL != nil {
		v.RelayURL = *patch.RelayURL
	}
	if patch.CLIPort != nil {
		v.CLIPort = *patch.CLIPort
	}
	if patch.AppLanguage != nil {
		v.AppLanguage = *patch.AppLanguage
	}
	if patch.Appearance != nil {
		v.Appearance = *patch.Appearance
	}
	if patch.SidebarWidth != nil {
		v.SidebarWidth = *patch.SidebarWidth
	}
	if patch.InspectorWidth != nil {
		v.InspectorWidth = *patch.InspectorWidth
	}
	if patch.SidebarCollapsed != nil {
		v.SidebarCollapsed = *patch.SidebarCollapsed
	}
	if patch.ShowsHiddenChats != nil {
		v.ShowsHiddenChats = *patch.ShowsHiddenChats
	}
	if patch.CollapsesOtherChats != nil {
		v.CollapsesOtherChats = *patch.CollapsesOtherChats
	}
	saved := *v
	if s.path != "" {
		data, err := json.MarshalIndent(saved, "", "  ")
		if err == nil {
			err = os.WriteFile(s.path+".tmp", data, 0o600)
		}
		if err == nil {
			err = os.Rename(s.path+".tmp", s.path)
		}
		if err != nil {
			s.mu.Unlock()
			return s.get(), err
		}
	}
	s.value = saved
	s.mu.Unlock()
	return s.get(), nil
}

// Prefs is this computer's settings, for the pages.
type Prefs struct{}

// All returns the preferences as they are now.
func (Prefs) All() Preferences { return prefs.get() }

// Set rejects incompatible ports before any preference write or reconnect.
func (Prefs) Set(patch PreferencesPatch) (Preferences, error) {
	before := prefs.get()
	after, err := prefs.update(patch)
	if err != nil {
		return before, err
	}
	if after.Appearance != before.Appearance {
		applyAppearance(after.Appearance)
	}
	PreferencesChanged.Broadcast(after)
	if after.AppLanguage != before.AppLanguage {
		syncNativeLanguage()
		if l10n.Set(after.AppLanguage, mygo.App.Locale()) {
			app.languageChanged()
		}
		if native.win != nil {
			native.win.Invalidate()
		}
	}
	if after.CLIPort != before.CLIPort && app.cli != nil {
		go app.cli.reconnect()
	}
	return after, nil
}

// applyAppearance forces light or dark for the windows, or follows the system.
func applyAppearance(appearance string) {
	switch appearance {
	case "light":
		mygo.Theme.SetSource(mygo.ThemeLight)
	case "dark":
		mygo.Theme.SetSource(mygo.ThemeDark)
	default:
		mygo.Theme.SetSource(mygo.ThemeSystem)
	}
}

// setPrefs adapts native action callbacks, which cannot return binding errors.
func setPrefs(patch PreferencesPatch) Preferences {
	value, err := (Prefs{}).Set(patch)
	if err != nil {
		log.Printf("preferences: %v", err)
	}
	return value
}
