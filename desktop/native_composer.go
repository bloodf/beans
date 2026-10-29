package main

import (
	"slices"

	"github.com/egoist/mygo/ui"
)

// Composer container for the opt-in native shell (adapted from Lorca
// composer.go resize habits: multiline TextArea with line bounds). Draft
// ownership and send fencing stay on NativeStore.

func (n *nativeDesktop) composerContainer(c *ui.Context, chatID string) {
	d := n.store.Draft(chatID)
	ui.Column(c).Gap(8).Label(L("Composer")).Children(func() {
		if d.ReplyTo != "" {
			ui.Text(c, "Replying to "+d.ReplyTo)
			if ui.Button(c, "Cancel reply").Clicked() {
				d.ReplyTo = ""
				d.Revision++
			}
		}
		for i, a := range d.Attachments {
			if ui.Button(c.Key(a.ID), "Remove "+a.Name).Clicked() {
				d.Attachments = append(d.Attachments[:i], d.Attachments[i+1:]...)
				d.Revision++
				break
			}
		}
		if ui.Button(c, L("Attach files")).Disabled(d.Sending).Clicked() {
			n.attach(chatID)
		}
		ui.Row(c).Gap(6).Wrap().Children(func() {
			for _, b := range n.store.Bots {
				picked := slices.Contains(d.Mentions, b.ID)
				label := "@" + b.Name
				if picked {
					label = "Remove " + label
				}
				if ui.Button(c.Key("mention-"+b.ID), label).Disabled(d.Sending).Clicked() {
					if picked {
						d.Mentions = slices.DeleteFunc(d.Mentions, func(id string) bool { return id == b.ID })
					} else {
						d.Mentions = append(d.Mentions, b.ID)
					}
					d.Revision++
				}
			}
		})
		// Lines(1, 8) keeps composer resize lag bounded (cf5feb91).
		input := ui.TextArea(c.Key(chatID+"-composer"), &d.Text).Lines(1, 8).Label(L("Message")).Placeholder(L("Message…")).Disabled(d.Sending)
		composing := input.Composing()
		input.HandleInput(func(ev ui.InputEvent) bool {
			if composing || ev.Kind != ui.InputKeyDown || ev.Key != ui.KeyEnter {
				return false
			}
			if (prefs.get().SendOnReturn && ev.Mods == 0) || (!prefs.get().SendOnReturn && ev.Mods == ui.Cmd) {
				n.send(chatID)
				return true
			}
			return false
		})
		if input.Changed() {
			d.Revision++
		}
		if d.Error != "" {
			ui.Text(c, d.Error)
		}
		ui.Row(c).Gap(8).Children(func() {
			for _, id := range n.store.Running {
				if id == chatID {
					if pushButton(c, L("Stop responding"), pushOptions{}).Clicked() {
						n.stop(chatID)
					}
					break
				}
			}
			if pushButton(c, L("Send"), pushOptions{Kind: buttonPrimary, Disabled: d.Sending}).Clicked() {
				n.send(chatID)
			}
		})
	})
}
