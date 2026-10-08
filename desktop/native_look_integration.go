package main

import (
	"context"
	"encoding/json"
	"time"

	"github.com/bloodf/beans/desktop/model"
)

func (n *nativeDesktop) openLook(bot model.NativeBot) {
	if !n.store.Connected {
		return
	}
	epoch, authority, client := n.store.Epoch, n.authority, app.cli
	var editor *nativeLookEditor
	editor = newNativeLookEditor(bot, func(method string, params json.RawMessage, complete func(json.RawMessage, error)) {
		go func() {
			ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
			defer cancel()
			data, err := client.requestBound(ctx, method, params, &authority)
			postMain(func() {
				if epoch != n.store.Epoch || n.look != editor {
					return
				}
				complete(data, err)
				if n.win != nil {
					n.win.Invalidate()
				}
			})
		}()
	})
	n.look = editor
}
