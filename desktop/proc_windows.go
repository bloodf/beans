//go:build windows

package main

import (
	"os"
	"os/exec"
	"runtime"
	"syscall"
	"unsafe"
)

// configureChild gives the CLI a console of its own that never shows. The commands it runs for
// bots share that console; a CLI with none would open a console window for each of them.
func configureChild(command *exec.Cmd) {
	command.SysProcAttr = &syscall.SysProcAttr{HideWindow: true}
}

// Windows uses ACL-based access checks rather than Unix uid/mode metadata.
func desktopControlOwner(info os.FileInfo) bool { return true }

func openDesktopControlFile(root *os.Root, name string, flags int) (*os.File, error) {
	return root.OpenFile(name, flags, 0o600)
}

var desktopControlSecurity = syscall.NewLazyDLL("advapi32.dll")
var desktopConvertSecurity = desktopControlSecurity.NewProc("ConvertStringSecurityDescriptorToSecurityDescriptorW")
var desktopReadDACL = desktopControlSecurity.NewProc("GetSecurityDescriptorDacl")
var desktopSetSecurity = desktopControlSecurity.NewProc("SetSecurityInfo")
var desktopControlKernel = syscall.NewLazyDLL("kernel32.dll")
var desktopFreeSecurity = desktopControlKernel.NewProc("LocalFree")
var desktopReopenControl = desktopControlKernel.NewProc("ReOpenFile")

// Apply the private DACL to the newly opened inode, not a replaceable pathname.
func secureDesktopControlFile(file *os.File) error {
	token, err := syscall.OpenCurrentProcessToken()
	if err != nil { return err }
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil { return err }
	sid, err := user.User.Sid.String()
	if err != nil { return err }
	text, err := syscall.UTF16PtrFromString("D:P(A;;FA;;;" + sid + ")")
	if err != nil { return err }
	var descriptor uintptr
	ok, _, callErr := desktopConvertSecurity.Call(uintptr(unsafe.Pointer(text)), 1, uintptr(unsafe.Pointer(&descriptor)), 0)
	runtime.KeepAlive(text)
	if ok == 0 { return callErr }
	defer desktopFreeSecurity.Call(descriptor)
	var present, defaulted uint32
	var acl uintptr
	ok, _, callErr = desktopReadDACL.Call(descriptor, uintptr(unsafe.Pointer(&present)), uintptr(unsafe.Pointer(&acl)), uintptr(unsafe.Pointer(&defaulted)))
	if ok == 0 { return callErr }
	if present == 0 || acl == 0 { return syscall.EINVAL }
	// Reopen the same inode with WRITE_DAC: ordinary write handles do not
	// necessarily include permission to change its security descriptor.
	handle, _, openErr := desktopReopenControl.Call(file.Fd(), 0x00040000, 7, 0)
	runtime.KeepAlive(file)
	if handle == ^uintptr(0) { return openErr }
	defer syscall.CloseHandle(syscall.Handle(handle))
	// SE_FILE_OBJECT; DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION.
	status, _, _ := desktopSetSecurity.Call(handle, 1, 0x80000004, 0, 0, acl, 0)
	runtime.KeepAlive(file)
	if status != 0 { return syscall.Errno(status) }
	return nil
}

// stopChild ends a CLI the app replaces (a new port). At quit the CLI is left to notice that its
// parent went (`--parent-pid`) and stop its commands itself: Windows has no SIGTERM to ask it.
func stopChild(command *exec.Cmd, restarting bool) {
	if restarting && command.Process != nil {
		_ = command.Process.Kill()
	}
}
