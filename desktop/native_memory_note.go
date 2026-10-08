package main

import (
	"encoding/json"
	"github.com/egoist/mygo/ui"
	"strings"
)

const nativeMemoryNoteMaxBytes = 256 * 1024

type memoryNoteForm struct {
	BotID, Text, Hash                                 string
	Loaded, Conflict, ConfirmReload, ConfirmOverwrite bool
	MaxLines, MaxBytes                                int
}

func (n *nativeDesktop) openMemoryNote(botID string) {
	f := &n.memoryPanel.Forms
	if f.Pending {
		return
	}
	f.serial++
	f.Note = &memoryNoteForm{BotID: botID}
	n.memoryPanel.Open = true
	n.reloadMemoryNote()
}
func (n *nativeDesktop) reloadMemoryNote() {
	f := &n.memoryPanel.Forms
	d := f.Note
	if d == nil {
		return
	}
	n.memoryCall("bots.memory", jsonBytes(map[string]string{"bot_id": d.BotID}), func(data json.RawMessage, err error) {
		if err != nil {
			f.Error = "Memory note could not be loaded. Your draft is unchanged."
			return
		}
		var reply struct {
			BotID string `json:"bot_id"`
			Index *struct {
				Text     *string `json:"text"`
				Hash     *string `json:"hash"`
				MaxLines int     `json:"max_lines"`
				MaxBytes int     `json:"max_bytes"`
			} `json:"index"`
		}
		if json.Unmarshal(data, &reply) != nil || reply.BotID != d.BotID || reply.Index == nil || reply.Index.Text == nil || reply.Index.Hash == nil || *reply.Index.Hash == "" || len(*reply.Index.Text) > nativeMemoryNoteMaxBytes || reply.Index.MaxLines <= 0 || reply.Index.MaxBytes <= 0 {
			f.Error = "Invalid memory note reply. Your draft is unchanged."
			return
		}
		d.Text = *reply.Index.Text
		d.Hash = *reply.Index.Hash
		d.MaxLines = reply.Index.MaxLines
		d.MaxBytes = reply.Index.MaxBytes
		d.Loaded = true
		d.Conflict = false
		d.ConfirmReload = false
		d.ConfirmOverwrite = false
	})
}
func (n *nativeDesktop) saveMemoryNote(overwrite bool) {
	f := &n.memoryPanel.Forms
	d := f.Note
	if d == nil || !d.Loaded {
		return
	}
	if len(d.Text) > nativeMemoryNoteMaxBytes {
		f.Error = "MEMORY.md exceeds the 256 KiB file limit."
		return
	}
	if overwrite && !d.ConfirmOverwrite {
		f.Error = "Confirm overwrite before saving."
		return
	}
	params := map[string]any{"bot_id": d.BotID, "text": d.Text}
	if !overwrite {
		params["expected_hash"] = d.Hash
	}
	d.ConfirmOverwrite = false
	n.memoryCall("bots.memory.write", jsonBytes(params), func(data json.RawMessage, err error) {
		if err != nil {
			d.Conflict = strings.Contains(err.Error(), "changed since")
			f.Error = "Memory note could not be saved. Your draft is unchanged."
			return
		}
		var reply struct {
			Hash string `json:"hash"`
		}
		if json.Unmarshal(data, &reply) != nil || reply.Hash == "" {
			f.Error = "Invalid memory save reply. Your draft is unchanged."
			return
		}
		f.Note = nil
		f.serial++
	})
}
func (n *nativeDesktop) memoryNoteView(c *ui.Context) {
	f := &n.memoryPanel.Forms
	d := f.Note
	ui.Text(c, nativeL("MEMORY.md")).Bold()
	if f.Error != "" {
		ui.Text(c, nativeL(f.Error))
	}
	if d.Loaded {
		ui.Text(c, nativeL("Each turn loads the first %d lines or %d bytes. File limit: 256 KiB.", d.MaxLines, d.MaxBytes))
		ui.TextArea(c, &d.Text).Lines(8, 24).Label(nativeL("Memory note")).Disabled(f.Pending)
		lines := 0
		if d.Text != "" {
			lines = strings.Count(d.Text, "\n") + 1
		}
		ui.Text(c, nativeL("Draft: %d lines · %d bytes", lines, len(d.Text)))
	}
	if f.Pending {
		ui.Text(c, nativeL("Working…"))
	}
	ui.Row(c).Gap(8).Children(func() {
		if ui.Button(c, nativeL("Save memory note")).Disabled(f.Pending || !d.Loaded).Clicked() {
			n.saveMemoryNote(false)
		}
		if ui.Button(c, nativeL("Reload memory note…")).Disabled(f.Pending).Clicked() {
			if d.Loaded {
				d.ConfirmReload = true
			} else {
				n.reloadMemoryNote()
			}
		}
		if ui.Button(c, nativeL("Cancel note")).Disabled(f.Pending).Clicked() {
			f.Note = nil
			f.serial++
		}
	})
	if d.Conflict {
		ui.Text(c, nativeL("MEMORY.md changed while you were editing. Reload discards your draft; overwrite replaces the current file."))
		if ui.Button(c, nativeL("Overwrite with mine…")).Disabled(f.Pending).Clicked() {
			d.ConfirmOverwrite = true
		}
	}
	if d.ConfirmReload {
		if ui.Button(c, nativeL("Confirm discard and reload")).Disabled(f.Pending).Clicked() {
			n.reloadMemoryNote()
		}
		if ui.Button(c, nativeL("Keep note draft")).Clicked() {
			d.ConfirmReload = false
		}
	}
	if d.ConfirmOverwrite {
		if ui.Button(c, nativeL("Confirm overwrite memory note")).Disabled(f.Pending).Clicked() {
			n.saveMemoryNote(true)
		}
		if ui.Button(c, nativeL("Keep current file")).Clicked() {
			d.ConfirmOverwrite = false
		}
	}
}
