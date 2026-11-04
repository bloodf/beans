package main

import (
	"strconv"

	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
)

// Chat container for the opt-in native shell: sidebar list + transcript body.
// Send/paging still go through NativeStore and the local CLI (#259).

func (n *nativeDesktop) chatsSidebar(c *ui.Context) {
	ui.Column(c).Grow(1).Padding(8, 12).Gap(8).Children(func() {
		ui.Text(c, L("Chats")).Bold().FontSize(13)
		ui.Scroll(c).Grow(1).Children(func() {
			if !n.store.Connected {
				return
			}
			for i := range n.store.Chats {
				chat := &n.store.Chats[i]
				if n.avatars != nil {
					ui.Box(c.Key("cluster:"+chat.ID)).Size(32, 32).Shrink(0).Children(func() {
						for _, slot := range nativeGroupAvatarSlots(chat.BotIDs, 32) {
							for _, b := range n.store.Bots {
								if b.ID == slot.BotID {
									ui.Box(c.Key(chat.ID+":"+b.ID)).Absolute().Left(slot.X).Top(slot.Y).Size(slot.Size, slot.Size).Children(func() {
										n.avatars.viewSlot(c, b, n.activity.state(b.ID, chat.ID), slot.Size, "sidebar:"+chat.ID+":"+b.ID)
									})
									n.fetchAvatar(b)
									break
								}
							}
						}
					})
				}
				label := n.store.Title(chat)
				if chat.UnreadCount > 0 {
					label = label + " (" + formatUnread(chat.UnreadCount) + ")"
				}
				if ui.Button(c.Key(chat.ID), label).Selected(n.store.Selected == chat.ID).Clicked() {
					n.store.SelectChat(chat.ID)
					n.shell.selectChat()
				}
			}
		})
		if ui.Button(c, L("Open full client")).Clicked() {
			app.showMainWindow()
		}
	})
}

func (n *nativeDesktop) chatContainer(c *ui.Context) {
	ui.Column(c).Grow(1).MinHeight(0).Padding(16).Gap(12).Children(func() {
		if n.look != nil {
			n.look.View(c)
			if n.look.Done() && n.look.CanDismiss() {
				n.look = nil
			}
			return
		}
		if n.store.HasOwnedIntent() && ui.Button(c, L("Save native drafts")).Clicked() {
			n.saveDrafts()
		}
		if len(n.memoryRecovery) > 0 {
			ui.Text(c, nativeL("Memory edits are retained for their original account. Reconnect that account to recover them."))
			if ui.Button(c, nativeL("Discard retained memory edits…")).Clicked() {
				n.confirmDiscardMemoryRecovery()
			}
		}
		if n.store.Error != "" {
			ui.Text(c, n.store.Error)
		}
		if n.store.HasOwnedIntent() && ui.Button(c, L("Discard native drafts…")).Clicked() {
			n.confirmDiscard()
		}
		if !n.store.Connected {
			ui.Text(c, L("The Beans CLI is not running"))
			if ui.Button(c, L("Reconnect")).Clicked() {
				app.cli.reconnect()
			}
			return
		}
		if ui.Button(c, L("Diagnostics…")).Clicked() {
			n.openDiagnostics()
		}
		if n.diagnostics != nil {
			n.diagnostics.View(c)
			if n.diagnostics.Done() {
				n.diagnostics = nil
			}
			return
		}
		if n.memory != nil && ui.Button(c, L("Memory connections")).Clicked() {
			n.loadMemory()
		}
		if n.memoryPanel.Open {
			n.memoryView(c)
			return
		}
		if !n.store.HasIdentity {
			ui.Text(c, L("Create or restore an identity in the full client."))
			if ui.Button(c, L("Continue in full client…")).Clicked() {
				app.showOnboarding()
			}
			return
		}
		chat := n.store.Chat(n.store.Selected)
		if chat == nil {
			ui.Text(c, L("Select a chat"))
			return
		}
		n.transcriptView(c, chat)
		n.composerContainer(c, chat.ID)
	})
}

func (n *nativeDesktop) transcriptView(c *ui.Context, chat *model.NativeChat) {
	if n.avatars != nil && len(chat.BotIDs) > 1 {
		ui.Box(c.Key("header-cluster:"+chat.ID)).Size(32, 32).Children(func() {
			for _, slot := range nativeGroupAvatarSlots(chat.BotIDs, 32) {
				for _, bot := range n.store.Bots {
					if bot.ID == slot.BotID {
						ui.Box(c.Key(bot.ID)).Absolute().Left(slot.X).Top(slot.Y).Size(slot.Size, slot.Size).Children(func() {
							n.avatars.viewSlot(c, bot, n.activity.state(bot.ID, chat.ID), slot.Size, "header:"+chat.ID+":"+bot.ID)
						})
						n.fetchAvatar(bot)
						break
					}
				}
			}
		})
	}
	scroll := ui.Scroll(c.Key(chat.ID+"-transcript")).Grow(1).Gap(12).Label(L("Transcript"))
	state := ui.Local(scroll, "scroll", func() ui.ScrollState { return ui.ScrollState{} })
	if state != nil {
		scroll.TrackScroll(state)
		n.shell.headerScrollY = state.Y
	}
	scroll.Children(func() {
		if chat.HasMore && ui.Button(c, "Load older messages").Disabled(n.store.LoadingOlder[chat.ID]).Clicked() {
			n.older(chat.ID)
		}
		for _, m := range chat.Messages {
			ui.Column(c.Key(m.ID)).Gap(4).Children(func() {
				author := m.Author.Kind
				for _, b := range n.store.Bots {
					if b.ID == m.Author.BotID {
						author = b.Name
						break
					}
				}
				if n.avatars != nil && m.Author.Kind == "bot" {
					for _, bot := range n.store.Bots {
						if bot.ID == m.Author.BotID {
							n.avatars.viewSlot(c, bot, n.activity.state(bot.ID, chat.ID), 28, "message:"+chat.ID+":"+m.ID)
							n.fetchAvatar(bot)
							if ui.Button(c, "Edit bot Look").Clicked() {
								n.openLook(bot)
							}
							break
						}
					}
				}
				ui.Text(c, author).Bold()
				if quote := m.Body.ReplyTo; quote != nil {
					ui.Text(c, "↩ "+quote.Text).FontSize(12)
				}
				ui.Text(c, m.DisplayText())
				if m.State.Error != "" {
					ui.Text(c, m.State.Error)
				}
				if m.Body.Kind == "text" && m.State.Kind == "complete" && ui.Button(c, "Reply").Clicked() {
					d := n.store.Draft(chat.ID)
					d.ReplyTo = m.ID
					d.Revision++
				}
			})
		}
	})
}

func formatUnread(count int) string {
	return strconv.Itoa(count)
}
