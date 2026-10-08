package main

import (
	"context"
	"encoding/json"
	"time"

	"github.com/egoist/mygo/ui"
)

type nativeMemoryPanel struct {
	Open        bool
	Loading     bool
	Connections json.RawMessage
	Error       string
}

func (n *nativeDesktop) loadMemory() {
	if n.memory == nil || !n.store.Connected {
		return
	}
	n.memoryPanel.Open = true
	n.memoryPanel.Loading = true
	epoch, authority, client := n.store.Epoch, n.authority, app.cli
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		data, err := client.requestBound(ctx, "memory.connections.list", nil, &authority)
		var masked json.RawMessage
		if err == nil {
			masked, err = n.memory.masked("connections", json.RawMessage(data))
		}
		postMain(func() {
			if epoch != n.store.Epoch {
				return
			}
			n.memoryPanel.Loading = false
			if err != nil {
				n.memoryPanel.Error = err.Error()
			} else {
				n.memoryPanel.Connections = masked
				n.memoryPanel.Error = ""
			}
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
func (n *nativeDesktop) memoryView(c *ui.Context) {
	ui.Text(c, "Memory connections").Bold().FontSize(20)
	if ui.Button(c, "Back to chat").Clicked() {
		n.memoryPanel.Open = false
	}
	if n.memoryPanel.Loading {
		ui.Text(c, "Loading…")
		return
	}
	if n.memoryPanel.Error != "" {
		ui.Text(c, n.memoryPanel.Error)
	}
	var data struct {
		Connections []struct {
			ID           string  `json:"id"`
			Name         string  `json:"name"`
			Backend      string  `json:"backend"`
			HasSecret    bool    `json:"has_secret"`
			Availability string  `json:"availability"`
			Reason       *string `json:"reason"`
		}
	}
	if json.Unmarshal(n.memoryPanel.Connections, &data) != nil {
		return
	}
	ui.Scroll(c).Grow(1).Children(func() {
		for _, connection := range data.Connections {
			ui.Column(c.Key(connection.ID)).Gap(4).Children(func() {
				ui.Text(c, connection.Name).Bold()
				ui.Text(c, connection.Backend)
				if connection.HasSecret {
					ui.Text(c, "Credential saved")
				}
				if connection.Availability == "blocked" && connection.Reason != nil {
					ui.Text(c, *connection.Reason)
				}
			})
		}
	})
	if ui.Button(c, "Reload memory connections").Clicked() {
		n.loadMemory()
	}
}
