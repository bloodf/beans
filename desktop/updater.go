package main

import (
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/plugins/updater"
)

// UpdaterState is what Settings › General shows about updates.
type UpdaterState struct {
	Version string `json:"version"`
	// LastCheck is when the app last looked, in Unix seconds; 0 is never.
	LastCheck          int64 `json:"lastCheck"`
	AutomaticChecks    bool  `json:"automaticChecks"`
	AutomaticDownloads bool  `json:"automaticDownloads"`
}

// UpdaterChanged tells Settings that a check went through, the update window's checkbox moved, or
// a switch did.
var UpdaterChanged = mygo.NewEvent[UpdaterState]("updater:changed")

// useUpdater gives the app MyGo's update window, as Sparkle gives the macOS app its own: a daily
// check in the background, the release notes of a new version with Install Update, Remind Me
// Later and Skip This Version, the download's progress, and the offer to relaunch. It installs
// signed updates, by delta from the running version when the release has one. The user's choices
// are this computer's, in updater.json. The window speaks the language picked in Settings, else
// the system's.
func useUpdater() {
	mygo.Use(updater.New(updater.Options{Language: prefs.get().AppLanguage}))
	updater.OnChange(func() { UpdaterChanged.Broadcast(Host{}.UpdaterState()) })
}

// UpdaterState returns what Settings shows about updates.
func (Host) UpdaterState() UpdaterState {
	state := UpdaterState{
		Version:            mygo.App.Version(),
		AutomaticChecks:    updater.AutomaticChecks(),
		AutomaticDownloads: updater.AutomaticDownloads(),
	}
	if last := updater.LastCheck(); !last.IsZero() {
		state.LastCheck = last.Unix()
	}
	return state
}

// CheckForUpdates opens the update window, which says what the check finds.
func (Host) CheckForUpdates() { updater.CheckForUpdates() }

// SetAutomaticUpdates turns the daily check and the automatic install on or off.
func (Host) SetAutomaticUpdates(checks, downloads bool) UpdaterState {
	updater.SetAutomaticChecks(checks)
	updater.SetAutomaticDownloads(downloads)
	return Host{}.UpdaterState()
}
