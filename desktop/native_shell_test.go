package main

import (
	"math"
	"testing"

	"github.com/egoist/mygo"
)

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

func TestWidenedResize(t *testing.T) {
	area := mygo.Rectangle{X: 0, Y: 40, Width: 1920, Height: 1000}
	rect := func(x, width int) mygo.Rectangle {
		return mygo.Rectangle{X: x, Y: 100, Width: width, Height: 760}
	}
	tests := []struct {
		name string
		r    mygo.Rectangle
		by   int
		want mygo.Rectangle
	}{
		{"with room", rect(200, 900), 281, rect(200, 1181)},
		{"at the right edge", rect(1020, 900), 281, rect(739, 1181)},
		{"near the right edge", rect(920, 900), 281, rect(739, 1181)},
		{"at the left edge", rect(0, 900), 261, rect(0, 1161)},
		{"wider than the room", rect(100, 1700), 281, rect(0, 1920)},
		{"filling the area", rect(0, 1920), 281, rect(0, 1920)},
		{"partly off the display", rect(1500, 900), 281, rect(1219, 1181)},
		{"off both sides", rect(-100, 2200), 261, rect(-100, 2200)},
		{"all the room", rect(300, 900), math.MaxInt32, rect(0, 1920)},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			if got := widened(test.r, area, test.by); got != test.want {
				t.Errorf("widened(%v, %d) = %v, want %v", test.r, test.by, got, test.want)
			}
		})
	}
}

func TestDividerBounds(t *testing.T) {
	prefs.mu.Lock()
	prefs.path = ""
	prefs.value = Preferences{ShowsInspector: true, SendOnReturn: true, ShowTimestamps: true, CLIPort: defaultCLIPort()}
	prefs.mu.Unlock()

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
