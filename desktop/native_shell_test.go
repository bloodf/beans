package main

import (
	"testing"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

func withShellPrefs(t *testing.T) {
	t.Helper()
	prefs.mu.Lock()
	prevPath, prev := prefs.path, prefs.value
	prefs.path = ""
	prefs.value = Preferences{ShowsInspector: true, SendOnReturn: true, ShowTimestamps: true, CLIPort: defaultCLIPort()}
	prefs.mu.Unlock()
	t.Cleanup(func() {
		prefs.mu.Lock()
		prefs.path, prefs.value = prevPath, prev
		prefs.mu.Unlock()
	})
}

func TestFitPanesResize(t *testing.T) {
	// Wide window: both panes keep their requested widths.
	s, i, fits := fitPanes(1180, 261, 281)
	if !fits || s != 261 || i != 281 {
		t.Fatalf("wide: got sidebar=%v inspector=%v fits=%v", s, i, fits)
	}
	// Narrow: inspector gives way first; remaining short then shrinks the sidebar.
	s, i, fits = fitPanes(800, 261, 281)
	if i != inspectorMin+1 || s != sidebarMin+1 || fits {
		t.Fatalf("narrow: got sidebar=%v inspector=%v fits=%v", s, i, fits)
	}
	available := float32(contentMinWidth + sidebarMin + 1 + inspectorMin + 1)
	s, i, fits = fitPanes(available, sidebarMin+1, inspectorMin+1)
	if !fits || s != sidebarMin+1 || i != inspectorMin+1 {
		t.Fatalf("at mins: got sidebar=%v inspector=%v fits=%v", s, i, fits)
	}
	s, i, fits = fitPanes(available-1, sidebarMin+1, inspectorMin+1)
	if fits || s != sidebarMin+1 || i != inspectorMin+1 {
		t.Fatalf("one short: got sidebar=%v inspector=%v fits=%v", s, i, fits)
	}
}

func TestDividerBounds(t *testing.T) {
	withShellPrefs(t)
	s := newNativeShellLayout()
	s.sidebarWidth = 260
	s.inspectorWidth = 280

	var drag dividerDrag
	s.applyDividerMove(&drag, true, 100)
	if s.sidebarWidth != sidebarMax {
		t.Fatalf("sidebar upper bound: got %v want %v", s.sidebarWidth, sidebarMax)
	}
	s.finishDivider(&drag, true)
	if prefs.get().SidebarWidth != int(sidebarMax) {
		t.Fatalf("sidebar width not saved: %d", prefs.get().SidebarWidth)
	}

	drag = dividerDrag{}
	s.sidebarWidth = 260
	s.applyDividerMove(&drag, true, float32(sidebarMin-260))
	if s.sidebarWidth != sidebarMin {
		t.Fatalf("sidebar lower bound: got %v want %v", s.sidebarWidth, sidebarMin)
	}

	drag = dividerDrag{}
	s.sidebarWidth = 260
	collapsed := s.applyDividerMove(&drag, true, -(260-sidebarMin/2+1))
	if !collapsed || !s.sidebarCollapsed || drag.active {
		t.Fatalf("collapse past half-min: collapsed=%v state=%v active=%v", collapsed, s.sidebarCollapsed, drag.active)
	}
	inspector := s.inspectorWidth
	s.finishDivider(&drag, true)
	if s.inspectorWidth != inspector {
		t.Fatal("sidebar collapse changed inspector width")
	}

	s.sidebarCollapsed = false
	drag = dividerDrag{}
	s.inspectorWidth = 280
	s.applyDividerMove(&drag, false, -100)
	if s.inspectorWidth != inspectorMax {
		t.Fatalf("inspector upper bound: got %v want %v", s.inspectorWidth, inspectorMax)
	}
	drag = dividerDrag{}
	s.inspectorWidth = 280
	s.applyDividerMove(&drag, false, 100)
	if s.inspectorWidth != inspectorMin {
		t.Fatalf("inspector lower bound: got %v want %v", s.inspectorWidth, inspectorMin)
	}
}

func TestHeaderState(t *testing.T) {
	withShellPrefs(t)
	s := newNativeShellLayout()
	s.mode = "chat"
	if s.headerEdgeVisible() {
		t.Fatal("edge visible at rest")
	}
	if !s.contentUnderHeader(true) {
		t.Fatal("chat content should run under the header")
	}
	if s.contentUnderHeader(false) {
		t.Fatal("empty chat should not run under the header")
	}
	s.headerHovered = true
	if !s.headerEdgeVisible() {
		t.Fatal("edge hidden while header hovered")
	}
	s.headerHovered = false
	s.headerScrollY = 12
	if !s.headerEdgeVisible() {
		t.Fatal("edge hidden while transcript scrolled")
	}
	s.mode = "settings"
	s.headerScrollY = 12
	if s.headerEdgeVisible() {
		t.Fatal("settings should not show the chat header edge")
	}
	if s.contentUnderHeader(true) {
		t.Fatal("settings content starts below the header")
	}

	s.selectSettings("general")
	s.selectSettings("advanced")
	if !s.canGoBack() || s.canGoForward() {
		t.Fatalf("history after two panes: back=%v forward=%v index=%d", s.canGoBack(), s.canGoForward(), s.paneIndex)
	}
	s.goBack()
	if s.settingsPane != "general" || !s.canGoForward() {
		t.Fatalf("goBack: pane=%q forward=%v", s.settingsPane, s.canGoForward())
	}
	s.goForward()
	if s.settingsPane != "advanced" {
		t.Fatalf("goForward: pane=%q", s.settingsPane)
	}
	s.selectChat()
	if s.mode != "chat" || len(s.paneHistory) != 0 {
		t.Fatal("selectChat did not clear settings history")
	}
}

func TestNativePaneShortcuts(t *testing.T) {
	withShellPrefs(t)
	oldNative, oldWiden := native, widenNativeWindow
	t.Cleanup(func() {
		native = oldNative
		widenNativeWindow = oldWiden
	})
	native = &nativeDesktop{store: model.NewNativeStore(), shell: newNativeShellLayout()}
	native.shell.lastWidth = 1180
	native.shell.mode = "chat"
	native.shell.sidebarCollapsed = false
	native.shell.userWantsInspector = true
	widenNativeWindow = func(*nativeDesktop, int) int { return 0 }

	tt := ui.NewTester(native.view, 1180, 760)
	tt.Key(ui.Cmd, ui.KeyB)
	if !native.shell.sidebarCollapsed {
		t.Fatal("Cmd+B did not collapse the sidebar")
	}
	tt.Key(ui.Cmd, ui.KeyB)
	if native.shell.sidebarCollapsed {
		t.Fatal("Cmd+B did not reopen the sidebar")
	}
	tt.Key(ui.Cmd|ui.Shift, ui.KeyB)
	if native.shell.userWantsInspector {
		t.Fatal("Cmd+Shift+B did not hide the inspector")
	}
	tt.Key(ui.Cmd|ui.Shift, ui.KeyB)
	if !native.shell.userWantsInspector {
		t.Fatal("Cmd+Shift+B did not show the inspector")
	}
	tt.Key(ui.Cmd, ui.KeyComma)
	if native.shell.mode != "settings" || native.shell.settingsPane != "general" {
		t.Fatalf("Cmd+, did not open settings: mode=%q pane=%q", native.shell.mode, native.shell.settingsPane)
	}
}

func TestOpenPaneNarrowWindow(t *testing.T) {
	withShellPrefs(t)
	oldNative, oldWiden := native, widenNativeWindow
	t.Cleanup(func() {
		native = oldNative
		widenNativeWindow = oldWiden
	})
	native = &nativeDesktop{store: model.NewNativeStore(), shell: newNativeShellLayout()}
	// Sidebar open, inspector closed: content-min + sidebar leaves no room for the inspector.
	native.shell.lastWidth = contentMinWidth + defaultSidebarWidth + 1
	native.shell.mode = "chat"
	native.shell.sidebarCollapsed = false
	native.shell.userWantsInspector = false
	native.shell.squeezesContent = false
	var widenedBy int
	widenNativeWindow = func(_ *nativeDesktop, by int) int {
		widenedBy = by
		return 0 // no room on the display
	}

	_, before := native.shell.panes(native.shell.lastWidth)
	if before != 0 {
		t.Fatalf("setup: inspector already visible (%v)", before)
	}

	native.toggleInspector()
	if !native.shell.userWantsInspector {
		t.Fatal("toggle did not set ShowsInspector")
	}
	if !native.shell.squeezesContent {
		t.Fatal("narrow open did not set squeezesContent")
	}
	if widenedBy != int(native.shell.inspectorWidth+1) {
		t.Fatalf("widen requested %d, want %d", widenedBy, int(native.shell.inspectorWidth+1))
	}
	_, after := native.shell.panes(native.shell.lastWidth)
	if after == 0 {
		t.Fatal("inspector stayed hidden after squeeze open")
	}

	// A second toggle hides it again without needing more width.
	native.toggleInspector()
	if native.shell.userWantsInspector {
		t.Fatal("second toggle did not clear ShowsInspector")
	}
	_, hidden := native.shell.panes(native.shell.lastWidth)
	if hidden != 0 {
		t.Fatal("inspector still visible after hide")
	}
}

func TestOpenPaneFitsWithoutSqueeze(t *testing.T) {
	withShellPrefs(t)
	oldNative, oldWiden := native, widenNativeWindow
	t.Cleanup(func() {
		native = oldNative
		widenNativeWindow = oldWiden
	})
	native = &nativeDesktop{store: model.NewNativeStore(), shell: newNativeShellLayout()}
	native.shell.lastWidth = 1180
	native.shell.mode = "chat"
	native.shell.sidebarCollapsed = false
	native.shell.userWantsInspector = false
	calls := 0
	widenNativeWindow = func(*nativeDesktop, int) int {
		calls++
		return 0
	}
	native.toggleInspector()
	if !native.shell.userWantsInspector || native.shell.squeezesContent || calls != 0 {
		t.Fatalf("wide open: wants=%v squeeze=%v widenCalls=%d", native.shell.userWantsInspector, native.shell.squeezesContent, calls)
	}
}
