package main

import (
	"github.com/bloodf/beans/desktop/l10n"
	"github.com/egoist/mygo/ui"
)

func firstNonEmpty(values ...string) string {
	for _, value := range values {
		if value != "" {
			return value
		}
	}
	return ""
}

type hoverButtonOptions struct {
	Symbol   string
	Size     float32
	Title    string
	Trailing string
	Tooltip  string
	Label    string
	Disabled bool
	Active   bool
}

// hoverButton is a borderless symbol button that fills under the pointer
// (adapted from Lorca controls.go).
func hoverButton(c *ui.Context, o hoverButtonOptions) ui.Element {
	p := nativeColors(c)
	b := ui.ButtonBase(c).Height(28).MinWidth(28).Radius(6).Gap(5).Justify(ui.Center).TextColor(p.Label2)
	if o.Title != "" {
		b.Padding(0, 8).FontSize(13)
	}
	if label := firstNonEmpty(o.Label, o.Tooltip, o.Title); label != "" {
		b.Label(label)
	}
	if o.Tooltip != "" {
		b.Tooltip(o.Tooltip)
	}
	switch {
	case o.Disabled:
		b.Disabled(true)
	case b.Pressed():
		b.Background(p.Pressed)
	case b.Hovered() || o.Active:
		b.Background(p.Hover)
	}
	size := o.Size
	if size == 0 {
		size = 16
	}
	b.Children(func() {
		if o.Symbol != "" {
			symbol(c, o.Symbol, size, 1.75)
		}
		if o.Title != "" {
			ui.Text(c, o.Title).SingleLine()
		}
		if o.Trailing != "" {
			symbol(c, o.Trailing, 10, 2.5)
		}
	})
	return b
}

type buttonKind int

const (
	buttonDefault buttonKind = iota
	buttonPrimary
	buttonDestructive
)

type pushOptions struct {
	Kind     buttonKind
	Small    bool
	Large    bool
	Disabled bool
	Tooltip  string
	Symbol   string
}

func pushButton(c *ui.Context, title string, o pushOptions) ui.Element {
	p := nativeColors(c)
	b := ui.ButtonBase(c).Height(24).Padding(0, 12).Radius(6).Gap(5).Justify(ui.Center).FontSize(13).TextColor(p.Label).Label(title)
	switch {
	case o.Small:
		b.Height(22).Padding(0, 10).FontSize(11.5).Radius(5)
	case o.Large:
		b.Height(32).Padding(0, 18)
	}
	fill := p.ButtonBG
	if o.Kind == buttonPrimary && !o.Disabled {
		fill = p.Accent
		b.TextColor(p.AccentText)
	} else if o.Kind == buttonDestructive {
		b.TextColor(p.Red)
	}
	switch {
	case o.Disabled:
		b.Disabled(true)
	case b.Pressed():
		if o.Kind == buttonPrimary {
			fill = fill.Mix(ui.RGB(0, 0, 0), 0.2)
		} else {
			fill = fill.Mix(p.Label, 0.08)
		}
	case b.Hovered():
		if o.Kind == buttonPrimary {
			fill = fill.Mix(ui.RGB(0, 0, 0), 0.12)
		} else {
			fill = fill.Mix(p.Label, 0.04)
		}
	}
	b.Background(fill)
	if o.Tooltip != "" {
		b.Tooltip(o.Tooltip)
	}
	b.Children(func() {
		if o.Symbol != "" {
			symbol(c, o.Symbol, 13, 2)
		}
		ui.Text(c, title).SingleLine()
	})
	return b
}

// L is the shell localization entry point (Beans branding in keys).
func L(key string, args ...any) string { return l10n.L(key, args...) }
