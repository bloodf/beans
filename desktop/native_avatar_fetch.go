package main

import (
	"context"
	"encoding/json"
	"os"
	"time"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

func (n *nativeDesktop) fetchAvatar(bot model.NativeBot) {
	a := n.avatars
	if a == nil || !n.store.Connected || isMock() {
		return
	}
	if a.epoch != n.store.Epoch {
		a.photos = map[string]*ui.Bitmap{}
		a.pending = map[string]bool{}
		a.epoch = n.store.Epoch
	}
	var attachment struct {
		ID string `json:"id"`
	}
	if json.Unmarshal(bot.Avatar, &attachment) != nil || attachment.ID == "" {
		return
	}
	key := bot.ID + ":" + attachment.ID
	if a.pending[key] {
		return
	}
	a.pending[key] = true
	epoch, authority, client := n.store.Epoch, n.authority, app.cli
	payload, _ := json.Marshal(map[string]any{"attachment": bot.Avatar})
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		data, err := client.requestBound(ctx, "files.path", payload, &authority)
		var result struct {
			Path string `json:"path"`
		}
		var bitmap *ui.Bitmap
		if err == nil {
			err = json.Unmarshal(data, &result)
		}
		if err == nil {
			var bytes []byte
			bytes, err = os.ReadFile(result.Path)
			if err == nil {
				bitmap, err = ui.DecodeBitmap(bytes)
			}
		}
		postMain(func() {
			if epoch != n.store.Epoch || n.avatars != a {
				return
			}
			// A photo replaced during the fetch cannot override the newer profile.
			for _, current := range n.store.Bots {
				if current.ID == bot.ID && string(current.Avatar) == string(bot.Avatar) {
					if err == nil {
						a.photos[key] = bitmap
					}
					break
				}
			}
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
