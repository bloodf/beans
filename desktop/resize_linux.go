//go:build linux

package main

import (
	"context"
	"sync"

	"github.com/ebitengine/purego"
	"github.com/egoist/mygo"
)

// A frameless GTK window has no border to resize it by, so the main window's page draws thin
// edges that hand the drag to the window manager, as a border would.
var (
	gtkWindowBeginResizeDrag func(window uintptr, edge, button, rootX, rootY int32, timestamp uint32)
	loadResize               sync.Once
)

// gdkEdges are GdkWindowEdge's values, by the edge's compass name.
var gdkEdges = map[string]int32{"nw": 0, "n": 1, "ne": 2, "w": 3, "e": 4, "sw": 5, "s": 6, "se": 7}

// StartResize resizes the calling window from one of its edges while the button is down; x and
// y are where the pointer went down, in screen coordinates.
func (Host) StartResize(ctx context.Context, edge string, x, y int) {
	win := mygo.CallerWindow(ctx)
	code, ok := gdkEdges[edge]
	if win == nil || !ok {
		return
	}
	loadResize.Do(func() {
		// MyGo has GTK loaded already; this opens the same library.
		if lib, err := purego.Dlopen("libgtk-3.so.0", purego.RTLD_NOW|purego.RTLD_GLOBAL); err == nil {
			purego.RegisterLibFunc(&gtkWindowBeginResizeDrag, lib, "gtk_window_begin_resize_drag")
		}
	})
	if gtkWindowBeginResizeDrag == nil {
		return
	}
	handle := win.NativeHandle()
	mygo.RunOnMain(func() {
		// Button 1, and GDK_CURRENT_TIME: the press is the one still held.
		gtkWindowBeginResizeDrag(handle, code, 1, int32(x), int32(y), 0)
	})
}
