package main

import (
	"github.com/bloodf/beans/desktop/model"
	"github.com/egoist/mygo/ui"
	"strings"
	"testing"
)

func TestNativeMemoryNoteLanguageSwitch(t *testing.T) {
	oldPrefs, oldLocale := prefs, nativeSystemLocale
	prefs = &prefsStore{value: Preferences{AppLanguage: "en"}}
	nativeSystemLocale = func() string { return "zh-CN" }
	defer func() { prefs = oldPrefs; nativeSystemLocale = oldLocale }()
	n := &nativeDesktop{store: model.NewNativeStore()}
	d := &memoryNoteForm{BotID: "b", Text: "one\ntwo", Loaded: true, Hash: "h", MaxLines: 200, MaxBytes: 24000, Conflict: true}
	n.memoryPanel.Forms.Note = d
	n.memoryPanel.Forms.Error = "Memory note could not be saved. Your draft is unchanged."
	tt := ui.NewTester(n.memoryNoteView, 1000, 1800)
	if !tt.HasText("Save memory note") || !tt.HasText("Draft: 2 lines · 7 bytes") {
		t.Fatal("English note missing")
	}
	prefs.mu.Lock()
	prefs.value.AppLanguage = "zh-Hans"
	prefs.mu.Unlock()
	tt.Frame()
	for _, label := range []string{"保存记忆笔记", "草稿：2 行 · 7 字节", "无法保存记忆笔记。草稿保持不变。", "每轮加载前 200 行或 24000 字节。文件大小限制：256 KiB。"} {
		if !tt.HasText(label) {
			t.Fatalf("missing %s", label)
		}
	}
	if tt.HasText("Save memory note") {
		t.Fatal("language change retained English")
	}
	if err := tt.Click("重新加载记忆笔记…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("确认丢弃并重新加载") {
		t.Fatal("reload confirmation not translated")
	}
	if err := tt.Click("保留笔记草稿"); err != nil {
		t.Fatal(err)
	}
	if err := tt.Click("用我的版本覆盖…"); err != nil {
		t.Fatal(err)
	}
	tt.Frame()
	if !tt.HasText("确认覆盖记忆笔记") {
		t.Fatal("overwrite confirmation not translated")
	}
	if err := tt.Click("保留当前文件"); err != nil {
		t.Fatal(err)
	}
	if d.Text != "one\ntwo" || d.ConfirmOverwrite || d.ConfirmReload {
		t.Fatal("language actions altered draft")
	}
	n.memoryPanel.Forms.Pending = true
	tt.Frame()
	if !tt.HasText("处理中…") {
		t.Fatal("pending untranslated")
	}
	prefs.mu.Lock()
	prefs.value.AppLanguage = ""
	prefs.mu.Unlock()
	if nativeL("Save memory note") != "保存记忆笔记" {
		t.Fatal("system language ignored")
	}
	detail := nativeL("This removes unsaved connection secrets, consent choices and note edits retained for disconnected accounts.")
	if !strings.Contains(detail, "笔记编辑") {
		t.Fatal("discard omits notes")
	}
	for _, key := range []string{"Memory note could not be loaded. Your draft is unchanged.", "Invalid memory note reply. Your draft is unchanged.", "MEMORY.md exceeds the 256 KiB file limit.", "Confirm overwrite before saving.", "Invalid memory save reply. Your draft is unchanged."} {
		n.memoryPanel.Forms.Error = key
		tt.Frame()
		if tt.HasText(key) || !tt.HasText(nativeL(key)) {
			t.Fatal("untranslated error")
		}
	}
}
