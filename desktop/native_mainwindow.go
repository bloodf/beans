package main

import (
	"github.com/bloodf/beans/desktop/l10n"
	"github.com/egoist/mygo/plugins/glass"
	"github.com/egoist/mygo/ui"
)

// Native main-window chrome: headers, dividers, sidebar/content/inspector
// layout adapted from Lorca commits 1fb21829…d94feac8. Content still binds to
// NativeStore from #259/#268 rather than a parallel store.

func (n *nativeDesktop) shellView(c *ui.Context) {
	applyNativeTheme(c)
	p := nativeColors(c)
	width, _ := c.Size()
	n.shell.noteWidth(width)
	if n.store.Selected != "" {
		n.shell.shownChat = true
	}
	if !n.store.HasIdentity && n.store.Connected {
		n.shell.mode = "onboarding"
	} else if n.shell.mode == "onboarding" && n.store.HasIdentity {
		n.shell.selectChat()
	}

	sidebar, inspector := n.shell.panes(width)
	title, subtitle := n.shellTitle()
	if n.win != nil {
		name := appName()
		windowTitle := name
		if title != "" && title != name {
			windowTitle = title + " - " + name
		}
		n.win.SetTitle(windowTitle)
	}

	vibrant := c.Vibrancy()
	if vibrant {
		c.Root().Background(ui.Transparent)
	}
	root := ui.Box(c).Fill()
	if !vibrant {
		root.Background(p.Content)
	}
	root.Children(func() {
		ui.Row(c).Fill().AlignItems(ui.Stretch).Children(func() {
			if !n.shell.sidebarCollapsed {
				pane := ui.Column(c).Width(sidebar)
				if !vibrant {
					pane.Background(p.Sidebar)
				}
				pane.Children(func() {
					n.paneHeader(c).Padding(0, 8).Gap(4).Children(func() { n.leadingButtons(c, false) })
					ui.Column(c).Grow(1).MinHeight(0).Children(func() {
						if n.shell.mode == "settings" {
							n.settingsSidebar(c)
						} else {
							n.chatsSidebar(c)
						}
					})
				})
				ui.Box(c).Width(1).Background(p.ToolbarLine)
			}
			ui.Column(c).Grow(1).MinWidth(0).Background(p.Content).Children(func() {
				hasChat := n.shell.mode == "chat" && n.store.Chat(n.store.Selected) != nil
				underHeader := n.shell.contentUnderHeader(hasChat)
				body := ui.Column(c).Grow(1).MinHeight(0)
				if !underHeader {
					body.Margin(headerHeight, 0, 0, 0).DrawOver(func(painter *ui.Painter, r ui.Rect) {
						painter.FillGradient(ui.Rect{X: r.X, Y: r.Y, W: r.W, H: 10}, ui.LinearGradient{From: p.Content, To: p.Content.Alpha(0), Angle: 180}, 0)
					})
				}
				body.Children(func() { n.shellContent(c) })
				var edge ui.Element
				if underHeader {
					edge = ui.Box(c).Absolute().Top(0).Left(0).Right(0).Height(headerHeight).PassThrough()
				}
				header := n.contentHeader(c, title, subtitle, inspector == 0)
				n.shell.headerHovered = header.Hovered()
				if underHeader {
					edge.Material(nativeHeaderEdge{scrollY: n.shell.headerScrollY, hovered: n.shell.headerHovered, background: p.Content})
				}
			})
			if inspector > 0 {
				ui.Box(c).Width(1).Background(p.ToolbarLine)
				ui.Column(c).Width(inspector).Background(p.Inspector).Children(func() {
					bar := c.TitleBar()
					n.paneHeader(c).Padding(0, max(10, bar.Right+8), 0, 12).Children(func() {
						ui.Spacer(c)
						n.inspectorToggle(c)
					})
					ui.Column(c).Grow(1).MinHeight(0).Padding(12).Children(func() {
						ui.Text(c, L("Inspector")).Bold()
						if chat := n.store.Chat(n.store.Selected); chat != nil {
							ui.Text(c, n.store.Title(chat))
						}
						if ui.Button(c, L("Open full client")).Clicked() {
							app.showMainWindow()
						}
					})
				})
			}
		})
		if !n.shell.sidebarCollapsed {
			n.divider(c, sidebar, true)
		}
		if inspector > 0 {
			n.divider(c, width-inspector-1, false)
		}
	})
}

func (n *nativeDesktop) shellTitle() (string, string) {
	switch n.shell.mode {
	case "settings":
		return settingsPaneTitle(n.shell.settingsPane), ""
	case "onboarding":
		return L("Onboarding"), ""
	}
	if chat := n.store.Chat(n.store.Selected); chat != nil {
		return n.store.Title(chat), ""
	}
	return appName(), ""
}

func (n *nativeDesktop) shellContent(c *ui.Context) {
	switch n.shell.mode {
	case "settings":
		n.settingsPage(c)
	case "onboarding":
		n.onboardingView(c)
	default:
		n.chatContainer(c)
	}
}

func (n *nativeDesktop) paneHeader(c *ui.Context) ui.Element {
	return ui.Row(c).Height(headerHeight).Gap(8).Padding(0, 12).MinWidth(0).DragWindow()
}

func (n *nativeDesktop) divider(c *ui.Context, x float32, sidebar bool) {
	key := "inspector-divider"
	if sidebar {
		key = "sidebar-divider"
	}
	h := ui.Box(c.Key(key)).Absolute().Top(0).Bottom(0).Left(x-3).Width(7).Cursor(ui.CursorResizeEW).Label(L("Divider"))
	dragging := ui.Local(h, "drag", func() dividerDrag { return dividerDrag{} })
	if dragging == nil {
		return
	}
	dx, _, held := h.Dragged()
	if held {
		n.shell.applyDividerMove(dragging, sidebar, dx)
		if n.win != nil {
			n.win.Invalidate()
		}
	} else if dragging.active {
		n.shell.finishDivider(dragging, sidebar)
	}
}

func (n *nativeDesktop) leadingButtons(c *ui.Context, inContent bool) {
	bar := c.TitleBar()
	row := ui.Row(c).Gap(4)
	if bar.Left > 0 {
		inset := bar.Left + 4
		if inContent {
			inset = bar.Left
		}
		row.Margin(0, 0, 0, inset)
	}
	row.Children(func() {
		if hoverButton(c, hoverButtonOptions{Symbol: "sidebar.leading", Tooltip: L("Toggle Sidebar (%@)", shortcutText("CmdOrCtrl+B")), Label: L("Toggle Sidebar")}).Clicked() {
			n.shell.toggleSidebar()
		}
		if n.shell.mode != "settings" {
			create := hoverButton(c, hoverButtonOptions{Symbol: "plus", Tooltip: L("Create"), Label: L("Create")})
			create.Menu(func(menu *ui.Menu) {
				if menu.Item(L("Create New Bot…")).Chosen() {
					runNativeCommand("newBot")
				}
				if menu.Item(L("Create Group Chat…")).Chosen() {
					runNativeCommand("newGroupChat")
				}
				menu.Separator()
				if menu.Item(L("Pair a Device…")).Chosen() {
					runNativeCommand("pairDevice")
				}
			})
		}
		if hoverButton(c, hoverButtonOptions{Symbol: "gearshape", Tooltip: L("Settings…"), Label: L("Settings")}).Clicked() {
			n.shell.selectSettings("general")
		}
	})
}

func (n *nativeDesktop) inspectorToggle(c *ui.Context) {
	if hoverButton(c, hoverButtonOptions{Symbol: "sidebar.trailing", Tooltip: L("Toggle Inspector (%@)", shortcutText("CmdOrCtrl+Shift+B")), Label: L("Toggle Inspector")}).Clicked() {
		n.shell.toggleInspector()
	}
}

func (n *nativeDesktop) contentHeader(c *ui.Context, title, subtitle string, rightmost bool) ui.Element {
	p := nativeColors(c)
	header := n.paneHeader(c.Key("content-header")).Absolute().Top(0).Left(0).Right(0)
	if rightmost {
		header.Padding(0, max(12, c.TitleBar().Right+8), 0, 12)
	}
	header.Children(func() {
		if n.shell.sidebarCollapsed {
			n.leadingButtons(c, true)
		}
		if n.shell.mode == "settings" {
			ui.Row(c).Gap(2).Children(func() {
				if hoverButton(c, hoverButtonOptions{Symbol: "chevron.left", Tooltip: L("Back"), Label: L("Back"), Disabled: !n.shell.canGoBack()}).Clicked() {
					n.shell.goBack()
				}
				if hoverButton(c, hoverButtonOptions{Symbol: "chevron.right", Tooltip: L("Forward"), Label: L("Forward"), Disabled: !n.shell.canGoForward()}).Clicked() {
					n.shell.goForward()
				}
			})
		}
		ui.Column(c).Shrink(1).MinWidth(0).Justify(ui.Center).Children(func() {
			ui.Text(c, title).FontSize(13).FontWeight(700).FixedLineHeight(16).SingleLine()
			if subtitle != "" {
				ui.Text(c, subtitle).FontSize(11).FixedLineHeight(14).TextColor(p.Label2).SingleLine()
			}
		})
		ui.Spacer(c)
		if n.shell.mode == "chat" && rightmost {
			n.inspectorToggle(c)
		}
	})
	return header
}

// nativeHeaderEdge frosts scrolled chat content under the header (a670d908).
type nativeHeaderEdge struct {
	scrollY    float32
	hovered    bool
	background ui.Color
}

func (e nativeHeaderEdge) PaintMaterial(p *ui.Painter, box ui.Rect, radii [4]float32) {
	if e.hovered || e.scrollY > 0 {
		glass.ScrollEdge{Hard: true, Background: e.background}.PaintMaterial(p, box, radii)
	}
}

func syncNativeLanguage() {
	p := prefs.get()
	_ = l10n.Set(p.AppLanguage, nativeSystemLocale())
}
