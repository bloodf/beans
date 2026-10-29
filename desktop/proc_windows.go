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
var desktopConvertSid = desktopControlSecurity.NewProc("ConvertStringSidToSidW")
var desktopReadDACL = desktopControlSecurity.NewProc("GetSecurityDescriptorDacl")
var desktopSetSecurity = desktopControlSecurity.NewProc("SetSecurityInfo")
var desktopSetNamedSecurity = desktopControlSecurity.NewProc("SetNamedSecurityInfoW")
var desktopLookupPrivilege = desktopControlSecurity.NewProc("LookupPrivilegeValueW")
var desktopAdjustPrivileges = desktopControlSecurity.NewProc("AdjustTokenPrivileges")
var desktopControlKernel = syscall.NewLazyDLL("kernel32.dll")
var desktopFreeSecurity = desktopControlKernel.NewProc("LocalFree")
var desktopReopenControl = desktopControlKernel.NewProc("ReOpenFile")
var desktopGetSecurity = desktopControlSecurity.NewProc("GetSecurityInfo")
var desktopValidSID = desktopControlSecurity.NewProc("IsValidSid")
var desktopSIDLength = desktopControlSecurity.NewProc("GetLengthSid")
var desktopValidACL = desktopControlSecurity.NewProc("IsValidAcl")
var desktopDescriptorLength = desktopControlSecurity.NewProc("GetSecurityDescriptorLength")
var desktopValidDescriptor = desktopControlSecurity.NewProc("IsValidSecurityDescriptor")

// OWNER | DACL | PROTECTED_DACL: exclusive create must name the current
// account as owner. Elevated Windows defaults (Administrators group owner)
// fail private admission even after a user-only DACL.
const desktopControlOwnerDACL = 0x80000005

// desktopControlPrivateSDDL is the protected DACL granting the account full
// control. extra appends ACE strings for fixtures (a foreign read grant).
func desktopControlPrivateSDDL(userSID, extra string) string {
	return "D:P(A;;FA;;;" + userSID + ")" + extra
}

func desktopControlCurrentUserSID() (string, error) {
	token, err := syscall.OpenCurrentProcessToken()
	if err != nil {
		return "", err
	}
	defer token.Close()
	user, err := token.GetTokenUser()
	if err != nil {
		return "", err
	}
	sid, err := user.User.Sid.String()
	runtime.KeepAlive(user)
	return sid, err
}

// desktopControlParseDACL converts an SDDL string to a DACL pointer. The
// caller must LocalFree descriptor when finished; acl is interior to it.
func desktopControlParseDACL(sddl string) (acl, descriptor uintptr, err error) {
	text, err := syscall.UTF16PtrFromString(sddl)
	if err != nil {
		return 0, 0, err
	}
	ok, _, callErr := desktopConvertSecurity.Call(uintptr(unsafe.Pointer(text)), 1, uintptr(unsafe.Pointer(&descriptor)), 0)
	runtime.KeepAlive(text)
	if ok == 0 {
		return 0, 0, callErr
	}
	var present, defaulted uint32
	ok, _, callErr = desktopReadDACL.Call(descriptor, uintptr(unsafe.Pointer(&present)), uintptr(unsafe.Pointer(&acl)), uintptr(unsafe.Pointer(&defaulted)))
	if ok == 0 {
		desktopFreeSecurity.Call(descriptor)
		return 0, 0, callErr
	}
	if present == 0 || acl == 0 {
		desktopFreeSecurity.Call(descriptor)
		return 0, 0, syscall.EINVAL
	}
	return acl, descriptor, nil
}

// desktopControlSIDFromString returns a LocalAlloc SID; caller LocalFrees it.
func desktopControlSIDFromString(sid string) (uintptr, error) {
	text, err := syscall.UTF16PtrFromString(sid)
	if err != nil {
		return 0, err
	}
	var pointer uintptr
	ok, _, callErr := desktopConvertSid.Call(uintptr(unsafe.Pointer(text)), uintptr(unsafe.Pointer(&pointer)))
	runtime.KeepAlive(text)
	if ok == 0 {
		return 0, callErr
	}
	return pointer, nil
}

// desktopControlEnableOwnershipPrivileges enables SeTakeOwnershipPrivilege and
// SeRestorePrivilege so OWNER_SECURITY_INFORMATION can rename Administrators-
// owned creates to the current account. restore disables them again.
func desktopControlEnableOwnershipPrivileges() (restore func(), err error) {
	process, err := syscall.GetCurrentProcess()
	if err != nil {
		return nil, err
	}
	var token syscall.Token
	if err := syscall.OpenProcessToken(process, syscall.TOKEN_ADJUST_PRIVILEGES|syscall.TOKEN_QUERY, &token); err != nil {
		return nil, err
	}
	type luid struct {
		low  uint32
		high int32
	}
	type entry struct {
		luid       luid
		attributes uint32
	}
	var privileges struct {
		count uint32
		privs [2]entry
	}
	for i, name := range []string{"SeTakeOwnershipPrivilege", "SeRestorePrivilege"} {
		text, err := syscall.UTF16PtrFromString(name)
		if err != nil {
			token.Close()
			return nil, err
		}
		ok, _, callErr := desktopLookupPrivilege.Call(0, uintptr(unsafe.Pointer(text)), uintptr(unsafe.Pointer(&privileges.privs[i].luid)))
		runtime.KeepAlive(text)
		if ok == 0 {
			token.Close()
			return nil, callErr
		}
		privileges.privs[i].attributes = 0x00000002 // SE_PRIVILEGE_ENABLED
	}
	privileges.count = 2
	ok, _, callErr := desktopAdjustPrivileges.Call(uintptr(token), 0, uintptr(unsafe.Pointer(&privileges)), 0, 0, 0)
	if ok == 0 {
		token.Close()
		return nil, callErr
	}
	return func() {
		for i := range privileges.privs {
			privileges.privs[i].attributes = 0
		}
		desktopAdjustPrivileges.Call(uintptr(token), 0, uintptr(unsafe.Pointer(&privileges)), 0, 0, 0)
		token.Close()
	}, nil
}

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

// Apply current-user owner and protected private DACL to the exclusively
// created inode before any secret is written. Existing objects are never
// repaired. Prefer the open handle; when elevated creates leave the
// Administrators group as owner (WRITE_DAC/WRITE_OWNER reopen fails), fall
// back to the pathname while the exclusive create handle is still held under
// the already-admitted private home.
func secureDesktopControlFile(file *os.File) error {
	sid, err := desktopControlCurrentUserSID()
	if err != nil {
		return err
	}
	acl, descriptor, err := desktopControlParseDACL(desktopControlPrivateSDDL(sid, ""))
	if err != nil {
		return err
	}
	defer desktopFreeSecurity.Call(descriptor)
	owner, err := desktopControlSIDFromString(sid)
	if err != nil {
		return err
	}
	defer desktopFreeSecurity.Call(owner)
	if restore, privErr := desktopControlEnableOwnershipPrivileges(); privErr == nil {
		defer restore()
	}
	apply := func(access uint32, info uint32, ownerSID uintptr) error {
		handle, _, openErr := desktopReopenControl.Call(file.Fd(), uintptr(access), 7, 0x02000000)
		runtime.KeepAlive(file)
		if handle == ^uintptr(0) {
			return openErr
		}
		defer syscall.CloseHandle(syscall.Handle(handle))
		status, _, _ := desktopSetSecurity.Call(handle, 1, uintptr(info), ownerSID, 0, acl, 0)
		runtime.KeepAlive(file)
		if status != 0 {
			return syscall.Errno(status)
		}
		return nil
	}
	// Prefer owner+DACL (WRITE_DAC|WRITE_OWNER). Elevated Windows often leaves
	// Administrators as owner; DACL-only then fails private admission.
	if err := apply(0x000C0000, desktopControlOwnerDACL, owner); err == nil {
		return nil
	}
	// Path fallback while the exclusive create handle is held under the
	// private home (GHA: rename Administrators-owned creates).
	pathPtr, err := syscall.UTF16PtrFromString(file.Name())
	if err == nil {
		status, _, _ := desktopSetNamedSecurity.Call(uintptr(unsafe.Pointer(pathPtr)), 1, desktopControlOwnerDACL, owner, 0, acl, 0)
		runtime.KeepAlive(pathPtr)
		runtime.KeepAlive(file)
		if status == 0 {
			return nil
		}
	}
	// Non-elevated creates are already user-owned; DACL-only matches prior path.
	return apply(0x00040000, 0x80000004, 0)
}

// stopChild ends a CLI the app replaces (a new port). At quit the CLI is left to notice that its
// parent went (`--parent-pid`) and stop its commands itself: Windows has no SIGTERM to ask it.
func stopChild(command *exec.Cmd, restarting bool) {
	if restarting && command.Process != nil {
		_ = command.Process.Kill()
	}
}
