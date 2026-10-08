package main

import "github.com/egoist/mygo"

// Local form values are user intent, not account authority. Retain them by the
// admitted account ID; never retain RPC results, consent handles or approvals.
func (n *nativeDesktop) suspendMemoryForms() {
	f := n.memoryPanel.Forms
	if (f.Connection != nil || f.Bot != nil) && n.store.AccountID != "" {
		if n.memoryRecovery == nil {
			n.memoryRecovery = map[string]memoryForms{}
		}
		f.Pending = false
		f.serial++
		f.Error = "Connection changed. Review recovered edits and renew approvals before saving."
		if b := f.Bot; b != nil {
			b.Draft = 0
			b.View = 0
			b.Approval = 0
			b.Preview = nil
			b.Target = nil
			b.Saved = nil
			b.Health = nil
			b.Operations = nil
			b.Confirm = false
			b.DeleteConfirm = false
			b.OperationCancel = false
			b.Recovering = true
		}
		n.memoryRecovery[n.store.AccountID] = f
	}
	n.memoryPanel = nativeMemoryPanel{}
}
func (n *nativeDesktop) recoverMemoryForms() {
	if !n.store.Connected || n.store.AccountID == "" {
		return
	}
	f, ok := n.memoryRecovery[n.store.AccountID]
	if !ok {
		return
	}
	delete(n.memoryRecovery, n.store.AccountID)
	n.memoryPanel = nativeMemoryPanel{Open: true, Forms: f}
	if f.Bot != nil {
		n.refreshBotMemory(false)
	}
}
func (n *nativeDesktop) hasMemoryIntent() bool {
	f := n.memoryPanel.Forms
	return f.Pending || f.Connection != nil || f.Bot != nil || len(n.memoryRecovery) != 0
}

func (n *nativeDesktop) confirmDiscardMemoryRecovery() {
	before := jsonBytes(n.memoryRecovery)
	epoch := n.store.Epoch
	parent := n.win
	go func() {
		answer, err := mygo.Dialog.Message(mygo.MessageOptions{Parent: parent, Message: "Discard retained memory edits?", Detail: "This removes unsaved connection secrets and consent choices retained for disconnected accounts.", Buttons: []string{"Keep edits", "Discard edits"}, CancelButton: 0})
		postMain(func() {
			if err != nil || answer.Button != 1 || epoch != n.store.Epoch || string(before) != string(jsonBytes(n.memoryRecovery)) {
				return
			}
			n.memoryRecovery = nil
			if n.win != nil {
				n.win.Invalidate()
			}
		})
	}()
}
