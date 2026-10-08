package main

import (
	"encoding/json"
	"fmt"
	"strconv"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

type avatarPath struct {
	Start    [2]float32   `json:"start"`
	Segments [][6]float32 `json:"segments"`
}
type avatarFrame struct {
	Background struct {
		Path    avatarPath `json:"path"`
		Fill    string     `json:"fill"`
		Opacity float32    `json:"opacity"`
	} `json:"background"`
	Core   avatarPath `json:"core"`
	Extra  avatarPath `json:"extra"`
	Body   [6]float32 `json:"body"`
	Petals []struct {
		X float32 `json:"cx"`
		Y float32 `json:"cy"`
		R float32 `json:"r"`
	} `json:"petals"`
	Head        string        `json:"head"`
	Eye         string        `json:"eye"`
	Eyes        [2]avatarPath `json:"eyes"`
	EyeMatrices [2][6]float32 `json:"eyeMatrices"`
}
type nativeAvatars struct {
	runtime *sharedRuntime
	frames  map[string]*avatarFrame
	photos  map[string]*ui.Bitmap
	pending map[string]bool
	epoch   uint64
	motion  map[string]*nativeAvatarMotion
	visible bool
}

func newNativeAvatars(r *sharedRuntime) *nativeAvatars {
	return &nativeAvatars{runtime: r, frames: map[string]*avatarFrame{}, photos: map[string]*ui.Bitmap{}, pending: map[string]bool{}}
}
func (a *nativeAvatars) frame(bot model.NativeBot, state string) (*avatarFrame, error) {
	look := bot.Look
	if len(look) == 0 {
		look = json.RawMessage("null")
	}
	key := bot.ID + ":" + string(look) + ":" + state
	if f := a.frames[key]; f != nil {
		return f, nil
	}
	var f avatarFrame
	err := a.runtime.eval(`return avatarFrame(botAvatarGeometry(input.id,input.look,input.state),0,0);`, map[string]any{"id": bot.ID, "look": look, "state": state}, &f)
	if err != nil {
		return nil, err
	}
	a.frames[key] = &f
	return &f, nil
}
func avatarColor(s string, opacity float32) ui.Color {
	if len(s) != 7 || s[0] != '#' {
		return ui.Color{}
	}
	v, err := strconv.ParseUint(s[1:], 16, 24)
	if err != nil {
		return ui.Color{}
	}
	return ui.RGBA(uint8(v>>16), uint8(v>>8), uint8(v), opacity)
}
func avatarContour(path avatarPath, m [6]float32, r ui.Rect) *ui.Path {
	point := func(x, y float32) (float32, float32) {
		return r.X + (m[0]*x+m[2]*y+m[4])*r.W/100, r.Y + (m[1]*x+m[3]*y+m[5])*r.H/100
	}
	var p ui.Path
	x, y := point(path.Start[0], path.Start[1])
	p.MoveTo(x, y)
	for _, s := range path.Segments {
		x1, y1 := point(s[0], s[1])
		x2, y2 := point(s[2], s[3])
		x3, y3 := point(s[4], s[5])
		p.CubeTo(x1, y1, x2, y2, x3, y3)
	}
	p.Close()
	return &p
}
func paintAvatar(p *ui.Painter, r ui.Rect, f *avatarFrame) {
	identity := [6]float32{1, 0, 0, 1, 0, 0}
	p.FillPath(avatarContour(f.Background.Path, identity, r), avatarColor(f.Background.Fill, f.Background.Opacity))
	head := avatarColor(f.Head, 1)
	p.FillPath(avatarContour(f.Core, f.Body, r), head)
	p.FillPath(avatarContour(f.Extra, f.Body, r), head)
	for _, petal := range f.Petals {
		if petal.R <= 0 {
			continue
		}
		// Convert a circle through the same affine matrix with cubics, preserving nonuniform scale.
		k := float32(.5522847498)
		x, y, rad := petal.X, petal.Y, petal.R
		contour := avatarPath{Start: [2]float32{x + rad, y}, Segments: [][6]float32{{x + rad, y + rad*k, x + rad*k, y + rad, x, y + rad}, {x - rad*k, y + rad, x - rad, y + rad*k, x - rad, y}, {x - rad, y - rad*k, x - rad*k, y - rad, x, y - rad}, {x + rad*k, y - rad, x + rad, y - rad*k, x + rad, y}}}
		p.FillPath(avatarContour(contour, f.Body, r), head)
	}
	for i := range f.Eyes {
		p.FillPath(avatarContour(f.Eyes[i], f.EyeMatrices[i], r), avatarColor(f.Eye, 1))
	}
}
func (a *nativeAvatars) view(c *ui.Context, bot model.NativeBot, state string, size float32) {
	var attachment struct {
		ID string `json:"id"`
	}
	_ = json.Unmarshal(bot.Avatar, &attachment)
	if photo := a.photos[bot.ID+":"+attachment.ID]; photo != nil {
		ui.Image(c, photo).Size(size, size).Radius(size / 2).Clip().Fit(ui.Cover).Label(bot.Name)
		return
	}
	f, err := a.frame(bot, state)
	if err != nil {
		ui.Text(c, fmt.Sprintf("Avatar: %v", err))
		return
	}
	if a.motion == nil {
		a.motion = map[string]*nativeAvatarMotion{}
	}
	controller := a.motion[bot.ID]
	if controller == nil {
		controller = newNativeAvatarMotion(a.runtime)
		a.motion[bot.ID] = controller
	}
	if err := controller.SetTarget(bot.ID, bot.Look, state); err != nil {
		ui.Text(c, err.Error())
		return
	}
	reduce := c.Preferences().ReduceMotion
	ui.Box(c).Size(size, size).Shrink(0).Role(ui.RoleImage).Label(bot.Name).Draw(func(p *ui.Painter, r ui.Rect) {
		frame, moving, err := controller.Frame(float64(p.Now().UnixMilli()), a.visible, reduce)
		if err != nil {
			paintAvatar(p, r, f)
			return
		}
		paintAvatar(p, r, &frame)
		if moving {
			p.AnimationFrame()
		}
	})
}

func (a *nativeAvatars) reset(epoch uint64) {
	a.photos = map[string]*ui.Bitmap{}
	a.pending = map[string]bool{}
	a.epoch = epoch
	for _, controller := range a.motion {
		controller.Reset()
	}
	a.motion = nil
}
