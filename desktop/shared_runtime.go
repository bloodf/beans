package main

import (
	_ "embed"
	"encoding/json"
	"fmt"
	"sync"

	"github.com/dop251/goja"
)

//go:embed assets/blobatar.js
var blobatarSource string

//go:embed assets/memory.js
var memorySource string

type sharedRuntime struct {
	mu sync.Mutex
	vm *goja.Runtime
}

func newSharedRuntime() (*sharedRuntime, error) {
	vm := goja.New()
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
