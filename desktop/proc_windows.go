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
var desktopGetSecurity = desktopControlSecurity.NewProc("GetSecurityInfo")
var desktopValidSID = desktopControlSecurity.NewProc("IsValidSid")
var desktopSIDLength = desktopControlSecurity.NewProc("GetLengthSid")
var desktopValidACL = desktopControlSecurity.NewProc("IsValidAcl")
var desktopDescriptorLength = desktopControlSecurity.NewProc("GetSecurityDescriptorLength")
var desktopValidDescriptor = desktopControlSecurity.NewProc("IsValidSecurityDescriptor")

func desktopSIDBytes(sid uintptr) ([]byte, error) {
	if sid == 0 { return nil, syscall.EINVAL }
	valid, _, _ := desktopValidSID.Call(sid)
	if valid == 0 { return nil, syscall.EINVAL }
	length, _, _ := desktopSIDLength.Call(sid)
	if length < 8 || length > 68 { return nil, syscall.EINVAL }
	return unsafe.Slice((*byte)(unsafe.Pointer(sid)), int(length)), nil
}

// Query the opened inode, never the replaceable pathname. This is admission,
// not ACL repair; even an administrator-group grant is not private to this user.
func validateDesktopControlObject(file *os.File, private bool) error {
	token, err := syscall.OpenCurrentProcessToken()
	if err != nil { return err }
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil { return err }
	userSID, err := desktopSIDBytes(uintptr(unsafe.Pointer(user.User.Sid)))
	if err != nil { return err }
	var owner, acl, descriptor uintptr
	status, _, _ := desktopGetSecurity.Call(file.Fd(), 1, 5,
		uintptr(unsafe.Pointer(&owner)), 0, uintptr(unsafe.Pointer(&acl)), 0,
		uintptr(unsafe.Pointer(&descriptor)))
	runtime.KeepAlive(file)
	if status != 0 { return syscall.Errno(status) }
	defer desktopFreeSecurity.Call(descriptor)
	if descriptor == 0 { return syscall.EINVAL }
	valid, _, _ := desktopValidDescriptor.Call(descriptor)
	if valid == 0 { return syscall.EINVAL }
	descriptorLength, _, _ := desktopDescriptorLength.Call(descriptor)
	within := func(pointer, length uintptr) bool {
		return pointer >= descriptor && length <= descriptorLength && pointer-descriptor <= descriptorLength-length
	}
	if !within(owner, 8) || !within(acl, 8) { return syscall.EACCES }
	ownerHeader := unsafe.Slice((*byte)(unsafe.Pointer(owner)), 8)
	ownerLength := uintptr(8 + 4*int(ownerHeader[1]))
	if !within(owner, ownerLength) { return syscall.EINVAL }
	ownerSID, err := desktopSIDBytes(owner)
	if err != nil { return err }
	header := unsafe.Slice((*byte)(unsafe.Pointer(acl)), 8)
	length := int(header[2]) | int(header[3])<<8
	if length < 8 || !within(acl, uintptr(length)) { return syscall.EINVAL }
	valid, _, _ = desktopValidACL.Call(acl)
	if valid == 0 { return syscall.EINVAL }
	if !desktopControlACLPrivate(ownerSID, userSID, unsafe.Slice((*byte)(unsafe.Pointer(acl)), length), private) {
		return syscall.EACCES
	}
	runtime.KeepAlive(user)
	return nil
}

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
	handle, _, openErr := desktopReopenControl.Call(file.Fd(), 0x00040000, 7, 0x02000000)
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
