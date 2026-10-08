package main

import (
	"fmt"
	"github.com/egoist/mygo"
	"strings"
)

// Read the existing preference at render time; no separate native language setting.
var nativeSystemLocale = func() string { return mygo.App.Locale() }

func nativeL(key string, args ...any) string {
	language := prefs.get().AppLanguage
	if language != "en" && language != "zh-Hans" {
		if strings.HasPrefix(strings.ToLower(nativeSystemLocale()), "zh") {
			language = "zh-Hans"
		} else {
			language = "en"
		}
	}
	text := key
	if language == "zh-Hans" {
		if translated, ok := nativeNoteChinese[key]; ok {
			text = translated
		}
	}
	if len(args) > 0 {
		return fmt.Sprintf(text, args...)
	}
	return text
}

var nativeNoteChinese = map[string]string{
	"MEMORY.md":        "MEMORY.md",
	"MEMORY.md for %s": "%s 的 MEMORY.md",
	"Memory note":      "记忆笔记",
	"Memory note could not be loaded. Your draft is unchanged.":            "无法加载记忆笔记。草稿保持不变。",
	"Invalid memory note reply. Your draft is unchanged.":                  "记忆笔记响应无效。草稿保持不变。",
	"MEMORY.md exceeds the 256 KiB file limit.":                            "MEMORY.md 超过 256 KiB 文件大小限制。",
	"Confirm overwrite before saving.":                                     "保存前请确认覆盖。",
	"Memory note could not be saved. Your draft is unchanged.":             "无法保存记忆笔记。草稿保持不变。",
	"Invalid memory save reply. Your draft is unchanged.":                  "记忆保存响应无效。草稿保持不变。",
	"Each turn loads the first %d lines or %d bytes. File limit: 256 KiB.": "每轮加载前 %d 行或 %d 字节。文件大小限制：256 KiB。",
	"Draft: %d lines · %d bytes":                                           "草稿：%d 行 · %d 字节",
	"Working…":                                                             "处理中…",
	"Save memory note":                                                     "保存记忆笔记",
	"Reload memory note…":                                                  "重新加载记忆笔记…",
	"Cancel note":                                                          "取消笔记编辑",
	"MEMORY.md changed while you were editing. Reload discards your draft; overwrite replaces the current file.": "编辑期间 MEMORY.md 已更改。重新加载会丢弃草稿；覆盖会替换当前文件。",
	"Overwrite with mine…":           "用我的版本覆盖…",
	"Confirm discard and reload":     "确认丢弃并重新加载",
	"Keep note draft":                "保留笔记草稿",
	"Confirm overwrite memory note":  "确认覆盖记忆笔记",
	"Keep current file":              "保留当前文件",
	"Discard retained memory edits?": "丢弃保留的记忆编辑？",
	"This removes unsaved connection secrets, consent choices and note edits retained for disconnected accounts.": "这将移除为已断开连接账户保留的未保存连接密钥、授权选择和笔记编辑。",
	"Keep edits":    "保留编辑",
	"Discard edits": "丢弃编辑",
	"Connection changed. Review recovered edits and renew approvals before saving.": "连接已更改。保存前请检查恢复的编辑并重新确认授权。",
	"Wait for account admission": "请等待账户连接确认",
}
