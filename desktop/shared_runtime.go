package main

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"net/url"
	"strings"
	"sync"

	"github.com/dop251/goja"
)

//go:embed assets/blobatar.js
var blobatarSource string

//go:embed assets/memory.js
var memorySource string

//go:embed assets/avatar-activity.js
var avatarActivitySource string

type sharedRuntime struct {
	mu sync.Mutex
	vm *goja.Runtime
}

func newSharedRuntime() (*sharedRuntime, error) {
	vm := goja.New()
	// Setup uses only these URL properties; parsing has no network side effects.
	if err := vm.Set("nativeParseURL", func(raw string) map[string]string {
		u, err := url.Parse(strings.TrimSpace(raw))
		if err != nil || u.Scheme == "" || u.Host == "" || strings.Contains(raw, "\\") {
			panic(vm.NewTypeError("invalid URL"))
		}
		username, password := "", ""
		if u.User != nil {
			username = u.User.Username()
			password, _ = u.User.Password()
		}
		return map[string]string{"protocol": strings.ToLower(u.Scheme) + ":", "username": username, "password": password, "hash": u.Fragment}
	}); err != nil {
		return nil, err
	}
	if _, err := vm.RunString(`globalThis.URL = class { constructor(raw) { Object.assign(this,nativeParseURL(String(raw))); } };`); err != nil {
		return nil, err
	}
	// The shared contracts use standard browser helpers; no I/O globals exist.
	_, err := vm.RunString(`globalThis.TextEncoder = class { encode(s) { const a=[]; for (const c of s) { let n=c.codePointAt(0); if(n>=0xD800&&n<=0xDFFF)n=0xFFFD; if(n<128)a.push(n);else if(n<2048)a.push(192|(n>>6),128|(n&63));else if(n<65536)a.push(224|(n>>12),128|((n>>6)&63),128|(n&63));else a.push(240|(n>>18),128|((n>>12)&63),128|((n>>6)&63),128|(n&63)); } return Uint8Array.from(a); } }; globalThis.structuredClone = v => JSON.parse(JSON.stringify(v));`)
	if err != nil {
		return nil, err
	}
	if _, err = vm.RunString(blobatarSource); err != nil {
		return nil, err
	}
	if _, err = vm.RunString(memorySource); err != nil {
		return nil, err
	}
	if _, err = vm.RunString(avatarActivitySource); err != nil {
		return nil, err
	}
	return &sharedRuntime{vm: vm}, nil
}
func (r *sharedRuntime) eval(expression string, input any, output any) error {
	r.mu.Lock()
	defer r.mu.Unlock()
	data, err := json.Marshal(input)
	if err != nil {
		return err
	}
	if err = r.vm.Set("nativeInputJSON", string(data)); err != nil {
		return err
	}
	v, err := r.vm.RunString(`JSON.stringify((function(input){` + expression + `})(JSON.parse(nativeInputJSON)))`)
	if err != nil {
		return err
	}
	if goja.IsUndefined(v) {
		return fmt.Errorf("shared contract returned no value")
	}
	return json.Unmarshal([]byte(v.String()), output)
}
