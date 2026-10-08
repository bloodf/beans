//go:build !windows

package main

import (
	"os"
	"os/exec"
	"syscall"
	"time"
)

func configureChild(command *exec.Cmd) {}

func desktopControlOwner(info os.FileInfo) bool {
	metadata, ok := info.Sys().(*syscall.Stat_t)
	return ok && (metadata.Uid == 0 || metadata.Uid == uint32(os.Geteuid()))
}

func openDesktopControlFile(root *os.Root, name string, flags int) (*os.File, error) {
	return root.OpenFile(name, flags|syscall.O_NOFOLLOW|syscall.O_NONBLOCK, 0o600)
}

func secureDesktopControlFile(file *os.File) error { return nil }

func validateDesktopControlObject(file *os.File, private bool) error {
	info, err := file.Stat()
	if err != nil { return err }
	mode := os.FileMode(0)
	if info.IsDir() { mode = 0o022 } else if private { mode = 0o077 }
	if !desktopControlOwner(info) || info.Mode().Perm()&mode != 0 { return syscall.EACCES }
	return nil
}

// stopChild sends the CLI SIGTERM, which stops the commands bots left running before it exits,
// and kills it if it is still there five seconds later.
func stopChild(command *exec.Cmd, restarting bool) {
	if command.Process == nil {
		return
	}
	_ = command.Process.Signal(syscall.SIGTERM)
	process := command.Process
	time.AfterFunc(5*time.Second, func() { _ = process.Kill() })
}
