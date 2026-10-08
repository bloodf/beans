package main

import (
	"crypto/rand"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo"
	"github.com/egoist/mygo/ui"
)

// Requests and completions are supplied by the account/connection-fenced owner
// and run on the main queue. The editor never opens a CLI connection.
type nativeLookEditor struct {
	bot                    model.NativeBot
	request                func(string, json.RawMessage, func(json.RawMessage, error))
	runtime                *sharedRuntime
	look                   json.RawMessage
	fields                 map[string]map[string]string
	selection              string
	photo                  *FileInfo
	photoID                string
	removePhoto            bool
	changed, pending, done bool
	errorText              string
}

func newNativeLookEditor(bot model.NativeBot, request func(string, json.RawMessage, func(json.RawMessage, error))) *nativeLookEditor {
	e := &nativeLookEditor{bot: bot, request: request, look: append(json.RawMessage(nil), bot.Look...), selection: "base", fields: map[string]map[string]string{}}
	var err error
	e.runtime, err = newSharedRuntime()
	if err != nil {
		e.errorText = err.Error()
	}
	return e
}
func (e *nativeLookEditor) HasIntent() bool {
	return !e.done && (e.changed || e.photo != nil || e.removePhoto || e.pending)
}
func (e *nativeLookEditor) CanDismiss() bool { return !e.pending }
func (e *nativeLookEditor) Done() bool       { return e.done }

func (e *nativeLookEditor) validate() error {
	for _, fields := range e.fields {
		if text := fields["hue"]; text != "" {
			hue, err := strconv.ParseFloat(text, 64)
			if err != nil || !(hue >= 0 && hue < 360) {
				return fmt.Errorf("Hue must be a number in [0, 360).")
			}
		}
	}
	if len(e.look) == 0 || string(e.look) == "null" {
		return nil
	}
	if e.runtime == nil {
		return fmt.Errorf("Avatar runtime is unavailable")
	}
	var result struct {
		OK    bool   `json:"ok"`
		Error string `json:"error"`
	}
	if err := e.runtime.eval(`return validateBotLook(input);`, e.look, &result); err != nil {
		return err
	}
	if !result.OK {
		return fmt.Errorf("%s", result.Error)
	}
	return nil
}
func (e *nativeLookEditor) appearance() map[string]any {
	var look map[string]any
	_ = json.Unmarshal(e.look, &look)
	if e.selection == "base" {
		if base, ok := look["base"].(map[string]any); ok {
			return base
		}
	} else if states, ok := look["states"].(map[string]any); ok {
		if state, ok := states[e.selection].(map[string]any); ok {
			return state
		}
	}
	return map[string]any{}
}
func (e *nativeLookEditor) edit(field, text string) {
	if e.pending || e.done {
		return
	}
	var look map[string]any
	_ = json.Unmarshal(e.look, &look)
	if look == nil {
		look = map[string]any{"version": 1, "base": map[string]any{}}
	}
	appearance := e.appearance()
	var value any = text
	if field == "hue" && text != "" {
		n, err := strconv.ParseFloat(text, 64)
		if err != nil {
			e.changed = true
			e.errorText = "Hue must be a number in [0, 360)."
			return
		}
		value = n
	}
	if field == "motion" {
		value = text == "true"
	}
	if strings.HasPrefix(field, "palette.") {
		key := strings.TrimPrefix(field, "palette.")
		palette, _ := appearance["palette"].(map[string]any)
		if palette == nil {
			palette = map[string]any{}
		}
		if text == "" {
			delete(palette, key)
		} else {
			palette[key] = strings.ToUpper(text)
		}
		if len(palette) == 0 {
			delete(appearance, "palette")
		} else {
			appearance["palette"] = palette
		}
	} else if text == "" {
		delete(appearance, field)
	} else {
		appearance[field] = value
	}
	if e.selection == "base" {
		look["base"] = appearance
	} else {
		states, _ := look["states"].(map[string]any)
		if states == nil {
			states = map[string]any{}
		}
		states[e.selection] = appearance
		look["states"] = states
	}
	raw, err := json.Marshal(look)
	e.changed = true
	if err != nil {
		e.errorText = err.Error()
		return
	}
	e.look = raw
	e.errorText = ""
}
func (e *nativeLookEditor) reset() {
	if e.pending || e.done {
		return
	}
	if e.selection == "base" {
		e.look = json.RawMessage("null")
		e.fields = map[string]map[string]string{}
	} else {
		var look map[string]any
		_ = json.Unmarshal(e.look, &look)
		if states, ok := look["states"].(map[string]any); ok {
			delete(states, e.selection)
			if len(states) == 0 {
				delete(look, "states")
			}
			e.look, _ = json.Marshal(look)
		}
		delete(e.fields, e.selection)
	}
	e.changed = true
	e.errorText = ""
}
func (e *nativeLookEditor) save() {
	if e.pending || e.done {
		return
	}
	e.errorText = ""
	if e.changed {
		if err := e.validate(); err != nil {
			e.errorText = err.Error()
			return
		}
	}
	if !e.HasIntent() {
		e.done = true
		return
	}
	params := map[string]any{"id": e.bot.ID}
	if e.changed {
		if len(e.look) == 0 {
			params["look"] = nil
		} else {
			params["look"] = e.look
		}
	}
	if e.photo != nil {
		params["avatar"] = map[string]any{"id": e.photoID, "path": e.photo.Path, "name": e.photo.Name, "mime": e.photo.Mime}
	} else if e.removePhoto {
		params["avatar"] = nil
	}
	raw, err := json.Marshal(params)
	if err != nil {
		e.errorText = err.Error()
		return
	}
	if e.request == nil {
		e.errorText = "Account request is unavailable"
		return
	}
	e.pending = true
	e.request("bots.update", raw, func(_ json.RawMessage, err error) {
		if !e.pending || e.done {
			return
		}
		e.pending = false
		if err != nil {
			e.errorText = err.Error()
			return
		}
		e.done = true
		e.errorText = ""
	})
}
func (e *nativeLookEditor) choosePhoto() {
	if e.pending || e.done {
		return
	}
	paths, err := mygo.Dialog.Open(mygo.OpenDialogOptions{Title: "Choose bot photo", Filters: []mygo.FileFilter{{Name: "Images", Extensions: []string{"png", "jpg", "jpeg", "gif", "webp", "bmp", "tif", "tiff"}}}})
	if err != nil {
		e.errorText = err.Error()
		return
	}
	if len(paths) == 0 || e.pending || e.done {
		return
	}
	file, err := (Files{}).PrepareAvatar(paths[0])
	if err != nil {
		e.errorText = err.Error()
		return
	}
	var id [16]byte
	if _, err = rand.Read(id[:]); err != nil {
		e.errorText = err.Error()
		return
	}
	e.photo = &file
	e.photoID = hex.EncodeToString(id[:])
	e.removePhoto = false
	e.errorText = ""
}

func (e *nativeLookEditor) View(c *ui.Context) {
	ui.Text(c, "Look · "+e.bot.Name).Bold().FontSize(20)
	if e.pending {
		ui.Text(c, "Saving…")
		return
	}
	ui.Text(c, "Photos take priority. Generated appearance and photo stay independent.")
	ui.Select(c, &e.selection, []string{"base", "idle", "thinking", "responding", "working", "waiting", "retry", "error"}).Label("Appearance")
	if e.runtime != nil {
		preview := e.look
		if len(preview) == 0 {
			preview = json.RawMessage("null")
		}
		state := e.selection
		if state == "base" {
			state = "idle"
			var look map[string]any
			_ = json.Unmarshal(preview, &look)
			if look != nil {
				delete(look, "states")
				preview, _ = json.Marshal(look)
			}
		}
		var frame avatarFrame
		if err := e.runtime.eval(`return avatarFrame(botAvatarGeometry(input.id,input.look,input.state),0,0);`, map[string]any{"id": e.bot.ID, "look": preview, "state": state}, &frame); err == nil {
			ui.Box(c).Size(96, 96).Role(ui.RoleImage).Label("Generated preview").Draw(func(p *ui.Painter, r ui.Rect) { paintAvatar(p, r, &frame) })
		}
	}
	values := e.fields[e.selection]
	if values == nil {
		values = map[string]string{}
		a := e.appearance()
		for k, v := range a {
			if k == "palette" {
				if palette, ok := v.(map[string]any); ok {
					for channel, color := range palette {
						values["palette."+channel] = fmt.Sprint(color)
					}
				}
			} else {
				values[k] = fmt.Sprint(v)
			}
		}
		e.fields[e.selection] = values
	}
	ui.Scroll(c).Grow(1).Children(func() {
		for _, field := range []struct {
			name    string
			choices []string
		}{
			{"shape", []string{"", "round", "organic", "boxy", "capsule", "nub", "cloud", "droplet", "hexagon", "sun", "triangle"}},
			{"expression", []string{"", "idle", "happy", "sad", "mad", "surprised", "wink", "sleepy", "smug", "unsure", "scared", "love", "shy", "sick", "thinking"}},
			{"background", []string{"", "none", "square", "circle", "squircle"}},
			{"tone", []string{"", "pastel", "pale", "mid", "deep", "bright", "ink"}},
			{"motion", []string{"", "true", "false"}},
		} {
			value := values[field.name]
			if ui.Select(c.Key(field.name), &value, field.choices).Label(field.name + " (empty inherits)").Changed() {
				values[field.name] = value
				e.edit(field.name, value)
			}
		}
		for _, field := range []string{"hue", "palette.head", "palette.eye", "palette.bg"} {
			value := values[field]
			if ui.TextInput(c.Key(field), &value).Label(field + " (empty inherits)").Changed() {
				values[field] = value
				e.edit(field, value)
			}
		}
		if ui.Button(c, "Restore defaults / Reset state").Clicked() {
			e.reset()
		}
		if ui.Button(c, "Choose photo…").Clicked() {
			e.choosePhoto()
		}
		if e.photo != nil {
			ui.Text(c, "Photo: "+e.photo.Name)
		}
		if ui.Button(c, "Remove photo").Clicked() {
			e.photo = nil
			e.photoID = ""
			e.removePhoto = true
			e.errorText = ""
		}
	})
	if e.errorText != "" {
		ui.Text(c, e.errorText)
	}
	if ui.Button(c, "Save").Clicked() {
		e.save()
	}
	if ui.Button(c, "Cancel").Clicked() {
		e.done = true
	}
}
