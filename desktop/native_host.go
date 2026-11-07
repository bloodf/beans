package main

import (
	"runtime"
	"unicode/utf16"

	"github.com/egoist/mygo"
)

// Native host helpers adapted from Lorca host.go. Badge, OpenExternal and
// fitUTF16 already live on Host / services.go (local-only CLI boundary).

func noticeTitleBody(title, subtitle, body string) (string, string) {
	if runtime.GOOS != "windows" {
		return title, body
	}
	title = fitUTF16(title, 63)
	room := 255
	if subtitle != "" {
		room -= len(utf16.Encode([]rune(subtitle))) + 1
	}
	return title, fitUTF16(body, room)
}

func appVersion() string {
	if version := mygo.App.Version(); version != "" {
		return version
	}
	return "dev"
}
