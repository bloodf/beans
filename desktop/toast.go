package main

import (
	"time"

	"github.com/egoist/mygo/ui"
)

// toast is a line at the foot of a pane for what an action did, gone after two seconds.
type toast struct {
	text    string
	isError bool
	// at is when it showed, set by the frame that first draws it.
	at time.Time
}

func (t *toast) show(text string, isError bool) {
	*t = toast{text: text, isError: isError}
}

// view draws it over the bottom of the element it is in, at most maxWidth wide.
func (t *toast) view(c *ui.Context, maxWidth float32) {
	if t.text == "" {
		return
	}
	if t.at.IsZero() {
		t.at = c.Now()
	}
	const shown = 2 * time.Second
	elapsed := c.Now().Sub(t.at)
	if elapsed >= shown {
		// Its last frame fades out as it goes.
		*t = toast{}
		return
	}
	c.After(shown - elapsed)
	// The inverse of the window, so it stands out from whatever it is over.
	p := colors(c)
	box := ui.Row(c.Key("toast")).Attach(ui.AnchorBottom, ui.AnchorBottom).Bottom(16).MaxWidth(maxWidth).Padding(8, 14).Gap(7).Radius(12).
		Background(p.Toast).Shadow(0, 6, 20, 0, p.Shadow).Role(ui.RoleStatus).
		Transition(ui.ElementTransition{Enter: &ui.Motion{}, Exit: &ui.Motion{}, Duration: 200 * time.Millisecond})
	box.Children(func() {
		if t.isError {
			ui.Row(c).TextColor(p.Orange).Children(func() { symbol(c, "exclamationmark.triangle.fill", 14, 1.8) })
		}
		ui.Text(c, t.text).FontSize(13).LineHeight(1.35).MaxLines(2).TextColor(p.ToastText)
	})
}
