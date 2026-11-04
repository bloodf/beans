package main

import "github.com/egoist/mygo/ui"

// Native shell colors match desktop/src/ui/theme.css (adapted from Lorca theme.go).
type nativePalette struct {
	Label, Label2, Label3         ui.Color
	Separator, Placeholder, Link  ui.Color
	Accent, AccentText            ui.Color
	Window, Sidebar, Inspector    ui.Color
	Content, ToolbarLine          ui.Color
	Hover, Pressed, ButtonBG      ui.Color
	Field, FieldBorder, Scrim     ui.Color
	ComposerField, ComposerBorder ui.Color
	ComposerPrimary               ui.Color
	ComposerPrimaryContent        ui.Color
	Red, Orange, Green            ui.Color
}

var nativeLight = nativePalette{
	Label:                  ui.RGBA(0, 0, 0, 0.86),
	Label2:                 ui.RGBA(0, 0, 0, 0.52),
	Label3:                 ui.RGBA(0, 0, 0, 0.3),
	Separator:              ui.RGBA(0, 0, 0, 0.1),
	Placeholder:            ui.RGBA(0, 0, 0, 0.3),
	Link:                   ui.Hex("#0068da"),
	Accent:                 ui.Hex("#007aff"),
	AccentText:             ui.Hex("#ffffff"),
	Window:                 ui.Hex("#ececed"),
	Sidebar:                ui.Hex("#f3f3f4"),
	Inspector:              ui.Hex("#f6f6f7"),
	Content:                ui.Hex("#ffffff"),
	ToolbarLine:            ui.RGBA(0, 0, 0, 0.09),
	Hover:                  ui.RGBA(0, 0, 0, 0.06),
	Pressed:                ui.RGBA(0, 0, 0, 0.12),
	ButtonBG:               ui.RGBA(0, 0, 0, 0.08),
	Field:                  ui.Hex("#ffffff"),
	FieldBorder:            ui.RGBA(0, 0, 0, 0.15),
	Scrim:                  ui.RGBA(0, 0, 0, 0.22),
	ComposerField:          ui.RGBA(255, 255, 255, 0.86),
	ComposerBorder:         ui.RGBA(0, 0, 0, 0.12),
	ComposerPrimary:        ui.Hex("#000000"),
	ComposerPrimaryContent: ui.Hex("#ffffff"),
	Red:                    ui.Hex("#ff3b30"),
	Orange:                 ui.Hex("#ff9500"),
	Green:                  ui.Hex("#34c759"),
}

var nativeDark = nativePalette{
	Label:                  ui.RGBA(255, 255, 255, 0.87),
	Label2:                 ui.RGBA(255, 255, 255, 0.55),
	Label3:                 ui.RGBA(255, 255, 255, 0.28),
	Separator:              ui.RGBA(255, 255, 255, 0.1),
	Placeholder:            ui.RGBA(255, 255, 255, 0.3),
	Link:                   ui.Hex("#4ea1ff"),
	Accent:                 ui.Hex("#0a84ff"),
	AccentText:             ui.Hex("#ffffff"),
	Window:                 ui.Hex("#262628"),
	Sidebar:                ui.Hex("#232325"),
	Inspector:              ui.Hex("#222224"),
	Content:                ui.Hex("#1c1c1c"),
	ToolbarLine:            ui.RGBA(255, 255, 255, 0.08),
	Hover:                  ui.RGBA(255, 255, 255, 0.08),
	Pressed:                ui.RGBA(255, 255, 255, 0.14),
	ButtonBG:               ui.RGBA(255, 255, 255, 0.1),
	Field:                  ui.Hex("#2c2c2e"),
	FieldBorder:            ui.RGBA(255, 255, 255, 0.18),
	Scrim:                  ui.RGBA(0, 0, 0, 0.45),
	ComposerField:          ui.RGBA(44, 44, 46, 0.92),
	ComposerBorder:         ui.RGBA(255, 255, 255, 0.14),
	ComposerPrimary:        ui.Hex("#ffffff"),
	ComposerPrimaryContent: ui.Hex("#000000"),
	Red:                    ui.Hex("#ff453a"),
	Orange:                 ui.Hex("#ff9f0a"),
	Green:                  ui.Hex("#30d158"),
}

func nativeColors(c *ui.Context) nativePalette {
	if c.Theme().Dark {
		return nativeDark
	}
	return nativeLight
}

// applyNativeTheme gives shell widgets the Beans palette over the system theme.
func applyNativeTheme(c *ui.Context) {
	base := c.Theme()
	p := nativeColors(c)
	t := *base
	t.Background = p.Content
	t.Text = p.Label
	t.TextMuted = p.Label2
	t.Border = p.FieldBorder
	t.Surface = p.ButtonBG
	t.SurfaceHover = p.ButtonBG.Mix(p.Label, 0.04)
	t.SurfacePressed = p.ButtonBG.Mix(p.Label, 0.08)
	t.Accent = p.Accent
	t.AccentHover = p.Accent.Mix(ui.RGB(0, 0, 0), 0.12)
	t.AccentPressed = p.Accent.Mix(ui.RGB(0, 0, 0), 0.2)
	t.AccentText = p.AccentText
	t.Danger = p.Red
	t.Warning = p.Orange
	t.Success = p.Green
	t.Selection = p.Accent.Alpha(0.32)
	t.Focus = p.Accent.Alpha(0.7)
	t.Scrollbar = p.Label3
	t.Radius = 6
	c.SetTheme(&t)
}
