package main

import (
	"encoding/json"
	"image"
	"image/color"
	"testing"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

func TestNativePhotoReplacementCannotReuseOldAttachment(t *testing.T) {
	r, err := newSharedRuntime()
	if err != nil {
		t.Fatal(err)
	}
	a := newNativeAvatars(r)
	bot := model.NativeBot{ID: "b", Name: "Bot", Avatar: json.RawMessage(`{"id":"old"}`)}
	photo := image.NewRGBA(image.Rect(0, 0, 2, 2))
	for y := 0; y < 2; y++ {
		for x := 0; x < 2; x++ {
			photo.SetRGBA(x, y, color.RGBA{255, 0, 0, 255})
		}
	}
	a.photos["b:old"] = ui.NewBitmap(photo)
	tt := ui.NewTester(func(c *ui.Context) { a.view(c, bot, "idle", 100) }, 120, 120)
	if p := tt.Image().RGBAAt(50, 50); p.R != 255 || p.G != 0 || p.B != 0 {
		t.Fatal("available uploaded photo did not win")
	}
	bot.Avatar = json.RawMessage(`{"id":"new"}`)
	tt.Frame()
	if p := tt.Image().RGBAAt(50, 50); p.R == 255 && p.G == 0 && p.B == 0 {
		t.Fatal("old photo survived attachment replacement")
	}
	a.reset(2)
	if len(a.photos) != 0 {
		t.Fatal("photo crossed account epoch")
	}
}
