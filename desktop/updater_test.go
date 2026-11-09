package main

import (
	"context"
	"encoding/json"
	"errors"
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
	"net/http"
	"net/http/httptest"
	"testing"
	"time"
)

func TestVerifyUpdateRelayFreshFormat(t *testing.T) {
	tests := []struct {
		name   string
		body   string
		wantOK bool
	}{
		{"fresh relay", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, true},
		{"future health extension", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1,"extension":{"enabled":true}}`, true},
		{"missing format", `{"ok":true,"service":"beans-relay","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"wrong format", `{"ok":true,"service":"beans-relay","format":"beans-v1","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"duplicate format", `{"ok":true,"service":"beans-relay","format":"beans-v1","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"old protocol", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":4,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"lowered floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":4,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"missing floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"unsupported floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":6,"min_protocol":6,"min_roster_protocol":6,"memory_config_version":1}`, false},
		{"lowered roster floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":4,"memory_config_version":1}`, false},
		{"missing memory capability", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5}`, false},
		{"unsupported memory capability", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":2}`, false},
		{"malformed protocol", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":"5","min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/v1/health" || r.Header.Get("Beans-Protocol") != "5" || r.Header.Get("Beans-Format") != "beans-v2" {
					w.WriteHeader(http.StatusUpgradeRequired)
					return
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(test.body))
			}))
			defer server.Close()
			err := verifyUpdateRelay(context.Background(), server.URL, 5)
			if (err == nil) != test.wantOK {
				t.Fatalf("verifyUpdateRelay() = %v, want success %v", err, test.wantOK)
			}
		})
	}
}

// updateCall is one request the app's updates made of the CLI.
type updateCall struct {
	method string
	params map[string]any
	reply  chan error
}

type updateTransport struct{ calls chan updateCall }

func (updateTransport) Reconnect() {}
func (u updateTransport) Request(method string, params any) (json.RawMessage, error) {
	var values map[string]any
	bytes, _ := json.Marshal(params)
	_ = json.Unmarshal(bytes, &values)
	call := updateCall{method, values, make(chan error, 1)}
	u.calls <- call
	return json.RawMessage("null"), <-call.reply
}

func nextUpdateCall(t *testing.T, calls <-chan updateCall, method string) updateCall {
	t.Helper()
	select {
	case call := <-calls:
		if call.method != method {
			t.Fatalf("asked %s %v, want %s", call.method, call.params, method)
		}
		return call
	case <-time.After(2 * time.Second):
		t.Fatalf("no %s", method)
	}
	return updateCall{}
}

func remoteUpdatesTester(t *testing.T, check func(context.Context) (*appRelease, error)) (*appUpdates, chan updateCall, chan struct{}) {
	calls, relaunched := make(chan updateCall, 10), make(chan struct{}, 1)
	demo := store
	t.Cleanup(func() { store = demo })
	store = model.NewStore(updateTransport{calls}, func(fn func()) { fn() }, false)
	u := &appUpdates{
		enabled:  func() bool { return true },
		version:  func() string { return "0.1.14" },
		auto:     func() bool { return true },
		check:    check,
		relaunch: func() { relaunched <- struct{}{} },
		now:      time.Now,
	}
	t.Cleanup(func() {
		if u.timer != nil {
			u.timer.Stop()
		}
	})
	return u, calls, relaunched
}

// The app says what its feed offers when its CLI answers, and a Device's request installs it and
// relaunches the app once the CLI says no bot is at work.
func TestRemoteAppUpdate(t *testing.T) {
	installed := make(chan struct{}, 1)
	release := &appRelease{version: "0.1.15", install: func(context.Context) error { installed <- struct{}{}; return nil }}
	u, calls, relaunched := remoteUpdatesTester(t, func(context.Context) (*appRelease, error) { return release, nil })

	go u.connected()
	report := nextUpdateCall(t, calls, "app.update_status")
	if report.params["version"] != "0.1.14" || report.params["latest"] != "0.1.15" || report.params["auto"] != true || report.params["state"] != nil {
		t.Fatalf("first report %v", report.params)
	}
	report.reply <- nil

	go u.install()
	report = nextUpdateCall(t, calls, "app.update_status")
	if report.params["state"] != "installing" || report.params["latest"] != "0.1.15" {
		t.Fatalf("installing %v", report.params)
	}
	report.reply <- nil
	<-installed
	report = nextUpdateCall(t, calls, "app.update_status")
	if report.params["state"] != "restarting" {
		t.Fatalf("installed %v", report.params)
	}
	report.reply <- nil
	// A second request while it waits does nothing more.
	u.install()
	wait := nextUpdateCall(t, calls, "app.relaunch_when_idle")
	select {
	case <-relaunched:
		t.Fatal("relaunched before the CLI said its bots are idle")
	case <-time.After(50 * time.Millisecond):
	}
	wait.reply <- nil
	select {
	case <-relaunched:
	case <-time.After(2 * time.Second):
		t.Fatal("never relaunched")
	}
}

// A CLI that restarts while the app waits on it drops the wait; the next one is asked again.
func TestRemoteAppUpdateWaitsAgainAfterTheCLIRestarts(t *testing.T) {
	u, calls, relaunched := remoteUpdatesTester(t, nil)
	u.state, u.checked = "restarting", time.Now()
	go u.connected()
	nextUpdateCall(t, calls, "app.update_status").reply <- nil
	nextUpdateCall(t, calls, "app.relaunch_when_idle").reply <- errors.New("The Beans CLI is not running")
	select {
	case <-relaunched:
		t.Fatal("relaunched without the CLI's word")
	case <-time.After(50 * time.Millisecond):
	}
	go u.connected()
	nextUpdateCall(t, calls, "app.update_status").reply <- nil
	nextUpdateCall(t, calls, "app.relaunch_when_idle").reply <- nil
	select {
	case <-relaunched:
	case <-time.After(2 * time.Second):
		t.Fatal("never relaunched")
	}
}

// A failed install says why and offers the update again; an install asked during a background
// check follows it.
func TestRemoteAppUpdateFailures(t *testing.T) {
	checks := make(chan chan *appRelease, 2)
	u, calls, _ := remoteUpdatesTester(t, func(context.Context) (*appRelease, error) {
		answer := make(chan *appRelease)
		checks <- answer
		return <-answer, nil
	})
	broken := &appRelease{version: "0.1.15", install: func(context.Context) error { return errors.New("disk full") }}
	go u.connected()
	background := <-checks
	u.install() // the check runs: queued
	background <- broken
	nextUpdateCall(t, calls, "app.update_status").reply <- nil
	(<-checks) <- broken
	report := nextUpdateCall(t, calls, "app.update_status")
	if report.params["state"] != "installing" {
		t.Fatalf("the queued install did not run: %v", report.params)
	}
	report.reply <- nil
	report = nextUpdateCall(t, calls, "app.update_status")
	if report.params["state"] != nil || report.params["error"] != "disk full" || report.params["latest"] != "0.1.15" {
		t.Fatalf("failed install %v", report.params)
	}
	report.reply <- nil
}

// A request that comes while a Device's install checks the feed is answered by that install: when
// it finds nothing newer, a later check that finds a version says so and installs nothing.
func TestRemoteAppUpdateRequestDuringItsOwnCheck(t *testing.T) {
	checks := make(chan chan *appRelease, 2)
	u, calls, _ := remoteUpdatesTester(t, func(context.Context) (*appRelease, error) {
		answer := make(chan *appRelease)
		checks <- answer
		return <-answer, nil
	})
	installed := make(chan struct{}, 1)
	newer := &appRelease{version: "0.1.15", install: func(context.Context) error { installed <- struct{}{}; return nil }}
	go u.install()
	own := <-checks
	u.install() // the same request again, while it checks
	own <- nil
	nextUpdateCall(t, calls, "app.update_status").reply <- nil

	u.mu.Lock()
	u.busy = true
	u.mu.Unlock()
	go u.refresh()
	(<-checks) <- newer
	report := nextUpdateCall(t, calls, "app.update_status")
	if report.params["latest"] != "0.1.15" || report.params["state"] != nil {
		t.Fatalf("daily check %v", report.params)
	}
	report.reply <- nil
	select {
	case <-checks:
		t.Fatal("an install nobody asked for checked the feed")
	case <-installed:
		t.Fatal("an install nobody asked for went in")
	case <-time.After(200 * time.Millisecond):
	}
}

// A Device that shows this one has the app look at its feed, unless it did in the last minutes.
func TestRemoteAppUpdateCheckOnDemand(t *testing.T) {
	checks := make(chan struct{}, 2)
	u, calls, _ := remoteUpdatesTester(t, func(context.Context) (*appRelease, error) {
		checks <- struct{}{}
		return &appRelease{version: "0.1.15"}, nil
	})
	u.checked = time.Now().Add(-time.Hour)
	go u.checkNow()
	<-checks
	report := nextUpdateCall(t, calls, "app.update_status")
	if report.params["latest"] != "0.1.15" {
		t.Fatalf("check on demand %v", report.params)
	}
	report.reply <- nil
	u.checkNow()
	select {
	case <-checks:
		t.Fatal("checked again minutes after the last check")
	case <-time.After(50 * time.Millisecond):
	}
}

func TestRemoteUpdateStagesWithoutInstallation(t *testing.T) {
	u := &beansUpdater{}
	up := &mygo.Update{Version: "1.2.3"}
	ready := &verifiedDesktopRelease{readyUpdateManifest: readyUpdateManifest{Version: "1.2.3", Protocol: 5}}
	if err := u.stageRemoteUpdate(context.Background(), up, ready); err != nil {
		t.Fatal(err)
	}
	if u.pending != up || u.pendingRelease != ready || u.pendingProtocol != 5 || !u.pendingManual || !u.relaunchAfterInstall {
		t.Fatal("remote staging lost the signed selection or guarded relaunch intent")
	}
	if u.lease != nil || u.quitApproved || u.installationQuitApproved || u.finishing {
		t.Fatal("staging bypassed quit/lease admission")
	}
}

func TestRemoteUpdateRejectsQuitRacesAndCanceledSelection(t *testing.T) {
	up := &mygo.Update{Version: "1.2.3"}
	ready := &verifiedDesktopRelease{readyUpdateManifest: readyUpdateManifest{Version: "1.2.3", Protocol: 5}}
	for _, u := range []*beansUpdater{
		{checkingQuit: true}, {quitApproved: true}, {installationQuitApproved: true}, {finishing: true},
	} {
		if err := u.stageRemoteUpdate(context.Background(), up, ready); err == nil {
			t.Fatal("accepted update during guarded quit")
		}
		if u.pending != nil || u.relaunchAfterInstall {
			t.Fatal("rejected update changed pending state")
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	cancel()
	u := &beansUpdater{}
	if err := u.stageRemoteUpdate(ctx, up, ready); !errors.Is(err, context.Canceled) {
		t.Fatalf("canceled staging: %v", err)
	}
	if err := u.stageRemoteUpdate(context.Background(), &mygo.Update{Version: "1.2.4"}, ready); err == nil {
		t.Fatal("accepted mismatched signed selection")
	}
	if u.pending != nil || u.relaunchAfterInstall {
		t.Fatal("failed selection mutated pending state")
	}
}
