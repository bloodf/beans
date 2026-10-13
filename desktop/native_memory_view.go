package main

import (
	"context"
	"encoding/json"
	"strconv"
	"time"

	"github.com/egoist/mygo/ui"
)

type nativeMemoryPanel struct {
	Open        bool
	Loading     bool
	Connections json.RawMessage
	Error       string
	Forms       memoryForms
}

func (n *nativeDesktop) loadMemory() {
	if n.memory == nil || !n.store.Connected {
		return
	}
	if isMock() {
		n.memoryPanel.Error = "Memory services are unavailable in demo mode."
		return
	}
	n.memoryPanel.Open = true
	n.memoryPanel.Loading = true
	epoch, authority := n.store.Epoch, n.authority
	go func() {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		data, err := n.memoryRequest(ctx, "memory.connections.list", nil, authority)
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
	if n.memoryPanel.Forms.Note != nil {
		n.memoryNoteView(c)
		return
	}
	if n.memoryPanel.Forms.Embedding != nil {
		n.embeddingFormView(c)
		return
	}
	if n.memoryPanel.Forms.Connection != nil || n.memoryPanel.Forms.Bot != nil {
		n.memoryFormView(c)
		return
	}
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
		Embeddings []memoryEmbeddingRow `json:"embeddings"`
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
				var row memoryConnectionRow
				for _, candidate := range n.memoryRows() {
					if candidate.ID == connection.ID {
						row = candidate
						break
					}
				}
				if ui.Button(c, "Edit connection").Clicked() {
					n.editMemoryConnection(&row)
				}
				if ui.Button(c, "Disconnect connection…").Disabled(n.memoryPanel.Forms.Pending).Clicked() {
					n.confirmMemoryDisconnect(row)
				}
			})
		}
		for _, profile := range data.Embeddings {
			ui.Column(c.Key("embedding:" + profile.ID)).Children(func() {
				ui.Text(c, profile.Model+" · "+profile.Revision+" · "+strconv.Itoa(profile.Dimensions))
				if profile.HasSecret {
					ui.Text(c, "Credential saved")
				}
				if ui.Button(c, "Edit embedding profile").Clicked() {
					n.editMemoryEmbedding(&profile)
				}
			})
		}
	})
	if ui.Button(c, "Add memory connection").Clicked() {
		n.editMemoryConnection(nil)
	}
	if ui.Button(c, "Add embedding profile").Clicked() {
		n.editMemoryEmbedding(nil)
	}
	for _, bot := range n.store.Bots {
		if ui.Button(c.Key("memory-"+bot.ID), "Memory for "+bot.Name).Clicked() {
			n.openBotMemory(bot.ID)
		}
		if ui.Button(c.Key("note-"+bot.ID), nativeL("MEMORY.md for %s", bot.Name)).Clicked() {
			n.openMemoryNote(bot.ID)
		}
	}
	if ui.Button(c, "Reload memory connections").Clicked() {
		n.loadMemory()
	}
}
