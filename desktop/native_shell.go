package main

// Layout constants match desktop/src/ui/MainWindow.tsx and the adapted Lorca
// mainwindow (1fb21829…d94feac8): side panes give way to content, then collapse.
// Window resize helpers `widened` / badge / OpenExternal stay in services.go (host).
const (
	contentMinWidth            = 460
	sidebarMin, sidebarMax     = 232, 340
	inspectorMin, inspectorMax = 268, 320
	headerHeight               = 52
	defaultSidebarWidth        = 260
	defaultInspectorWidth      = 280
)

func clampShell(value, low, high float32) float32 {
	return min(high, max(low, value))
}

// fitPanes returns side-pane widths (including each divider DIP) for a window
// `available` wide. The inspector gives way first, then the sidebar.
func fitPanes(available, sidebar, inspector float32) (float32, float32, bool) {
	short := contentMinWidth - (available - sidebar - inspector)
	giveWay := func(width, low float32) float32 {
		taken := max(0, min(short, width-low))
		short -= taken
		return width - taken
	}
	if inspector > 0 {
		inspector = giveWay(inspector, inspectorMin+1)
	}
	if sidebar > 0 {
		sidebar = giveWay(sidebar, sidebarMin+1)
	}
	return sidebar, inspector, short <= 0
}

// nativeShellLayout holds the split-view widths and header presentation for the
// opt-in native window. Preferences persist widths and collapse the same way as
// the Solid page.
type nativeShellLayout struct {
	sidebarCollapsed   bool
	userWantsInspector bool
	squeezesContent    bool
	lastWidth          float32
	sidebarWidth       float32
	inspectorWidth     float32
	shownChat          bool
	// mode is "chat", "settings", or "onboarding".
	mode string
	// settingsPane is the settings content while mode == "settings".
	settingsPane string
	paneHistory  []string
	paneIndex    int
	walkingHist  bool
	// headerScrollY is the transcript scroll offset used by the chat header edge.
	headerScrollY float32
	headerHovered bool
}

func newNativeShellLayout() nativeShellLayout {
	p := prefs.get()
	s := nativeShellLayout{
		userWantsInspector: p.ShowsInspector,
		sidebarCollapsed:   p.SidebarCollapsed,
		sidebarWidth:       defaultSidebarWidth,
		inspectorWidth:     defaultInspectorWidth,
		mode:               "chat",
		settingsPane:       "general",
	}
	if p.SidebarWidth > 0 {
		s.sidebarWidth = clampShell(float32(p.SidebarWidth), sidebarMin, sidebarMax)
	}
	if p.InspectorWidth > 0 {
		s.inspectorWidth = clampShell(float32(p.InspectorWidth), inspectorMin, inspectorMax)
	}
	return s
}

func (s *nativeShellLayout) ensureDefaults() {
	if s.sidebarWidth == 0 {
		s.sidebarWidth = defaultSidebarWidth
	}
	if s.inspectorWidth == 0 {
		s.inspectorWidth = defaultInspectorWidth
	}
	if s.mode == "" {
		s.mode = "chat"
	}
	if s.settingsPane == "" {
		s.settingsPane = "general"
	}
}

// panes are the side panes' widths without the divider DIP, matching Solid.
func (s *nativeShellLayout) panes(available float32) (sidebar, inspector float32) {
	s.ensureDefaults()
	if !s.sidebarCollapsed {
		sidebar = s.sidebarWidth + 1
	}
	if s.showsInspector() {
		inspector = s.inspectorWidth + 1
	}
	side, insp, fits := fitPanes(available, sidebar, inspector)
	if !fits && inspector > 0 && !s.squeezesContent {
		side, insp, _ = fitPanes(available, sidebar, 0)
	}
	return max(0, side-1), max(0, insp-1)
}

func (s *nativeShellLayout) showsInspector() bool {
	if !s.userWantsInspector || s.mode != "chat" {
		return false
	}
	return true
}

func (s *nativeShellLayout) setSidebarCollapsed(collapsed bool) {
	s.sidebarCollapsed = collapsed
	prefs.update(PreferencesPatch{SidebarCollapsed: &collapsed})
}

func (s *nativeShellLayout) setInspectorWanted(wants bool) {
	s.userWantsInspector = wants
	prefs.update(PreferencesPatch{ShowsInspector: &wants})
}

// dividerDrag tracks a captured pane-divider drag (cf5feb91 / 68364c8e).
type dividerDrag struct {
	active bool
	from   float32
	offset float32
}

// applyDividerMove updates pane width while the pointer is held. Dragging the
// sidebar past half its minimum collapses it and clears the drag so the
// inspector handle does not inherit capture (68364c8e).
func (s *nativeShellLayout) applyDividerMove(drag *dividerDrag, sidebar bool, dx float32) (collapsed bool) {
	if !drag.active {
		drag.active = true
		drag.offset = 0
		if sidebar {
			drag.from = s.sidebarWidth
		} else {
			drag.from = s.inspectorWidth
		}
	}
	drag.offset += dx
	if sidebar {
		wanted := drag.from + drag.offset
		if wanted < sidebarMin/2 {
			drag.active = false
			s.setSidebarCollapsed(true)
			return true
		}
		s.sidebarWidth = clampShell(wanted, sidebarMin, sidebarMax)
		return false
	}
	s.inspectorWidth = clampShell(drag.from-drag.offset, inspectorMin, inspectorMax)
	return false
}

func (s *nativeShellLayout) finishDivider(drag *dividerDrag, sidebar bool) {
	if !drag.active {
		return
	}
	drag.active = false
	if sidebar {
		width := int(s.sidebarWidth)
		prefs.update(PreferencesPatch{SidebarWidth: &width})
		return
	}
	width := int(s.inspectorWidth)
	prefs.update(PreferencesPatch{InspectorWidth: &width})
}

// headerEdgeVisible is the hard scroll edge under a chat header (a670d908):
// frosted once the transcript scrolls or the pointer is over the header.
func (s *nativeShellLayout) headerEdgeVisible() bool {
	return s.mode == "chat" && (s.headerHovered || s.headerScrollY > 0)
}

// contentUnderHeader is whether the chat transcript runs under the absolute header.
func (s *nativeShellLayout) contentUnderHeader(hasChat bool) bool {
	return s.mode == "chat" && hasChat
}

func (s *nativeShellLayout) selectSettings(pane string) {
	entering := s.mode != "settings"
	s.mode = "settings"
	if pane == "" {
		pane = "general"
	}
	s.settingsPane = pane
	if s.walkingHist {
		return
	}
	if entering {
		s.paneHistory = []string{pane}
		s.paneIndex = 0
		return
	}
	kept := append([]string{}, s.paneHistory[:min(len(s.paneHistory), s.paneIndex+1)]...)
	if len(kept) == 0 || kept[len(kept)-1] != pane {
		kept = append(kept, pane)
	}
	s.paneHistory, s.paneIndex = kept, len(kept)-1
}

func (s *nativeShellLayout) selectChat() {
	s.mode = "chat"
	s.paneHistory, s.paneIndex = nil, 0
}

func (s *nativeShellLayout) selectOnboarding() {
	s.mode = "onboarding"
	s.paneHistory, s.paneIndex = nil, 0
}

func (s *nativeShellLayout) canGoBack() bool    { return s.paneIndex > 0 }
func (s *nativeShellLayout) canGoForward() bool { return s.paneIndex < len(s.paneHistory)-1 }

func (s *nativeShellLayout) goBack() {
	if !s.canGoBack() {
		return
	}
	s.paneIndex--
	s.walkingHist = true
	s.settingsPane = s.paneHistory[s.paneIndex]
	s.walkingHist = false
}

func (s *nativeShellLayout) goForward() {
	if !s.canGoForward() {
		return
	}
	s.paneIndex++
	s.walkingHist = true
	s.settingsPane = s.paneHistory[s.paneIndex]
	s.walkingHist = false
}

func (s *nativeShellLayout) noteWidth(width float32) {
	if width < s.lastWidth {
		s.squeezesContent = false
	}
	s.lastWidth = width
}
