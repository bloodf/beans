package main

import (
	"fmt"
	"sync"

	"github.com/egoist/mygo/ui"
)

// SF Symbol names draw Lucide shapes (adapted from Lorca icons.go). A name with
// no icon draws sparkles.
var filledSymbols = map[string]bool{"stop.fill": true, "pin.fill": true}

type symbolKey struct {
	name   string
	stroke float32
}

var symbols sync.Map // symbolKey → *ui.SVG

func symbolSVG(name string, stroke float32) *ui.SVG {
	key := symbolKey{name, stroke}
	if found, ok := symbols.Load(key); ok {
		return found.(*ui.SVG)
	}
	shape, ok := lucideShapes[symbolIcons[name]]
	if !ok {
		shape = lucideShapes["Sparkles"]
	}
	fill := "none"
	if filledSymbols[name] {
		fill = "currentColor"
	}
	svg := ui.MustParseSVG(fmt.Appendf(nil,
		`<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="%s" stroke="currentColor" stroke-width="%g" stroke-linecap="round" stroke-linejoin="round">%s</svg>`,
		fill, stroke, shape))
	symbols.Store(key, svg)
	return svg
}

func symbol(c *ui.Context, name string, size, stroke float32) ui.Element {
	return ui.Icon(c, symbolSVG(name, stroke)).Size(size, size)
}

var symbolIcons = map[string]string{
	"sidebar.leading":    "PanelLeft",
	"sidebar.trailing":   "PanelRight",
	"plus":               "Plus",
	"chevron.left":       "ChevronLeft",
	"chevron.right":      "ChevronRight",
	"xmark":              "X",
	"checkmark":          "Check",
	"gearshape":          "Settings",
	"magnifyingglass":    "Search",
	"info.circle":        "Info",
	"square.and.pencil":  "SquarePen",
	"trash":              "Trash2",
	"pin":                "Pin",
	"pin.fill":           "Pin",
	"stop.fill":          "Square",
	"paperclip":          "Paperclip",
	"ellipsis":           "Ellipsis",
	"person.2":           "Users",
	"arrow.right":        "ArrowRight",
	"sparkles":           "Sparkles",
}
