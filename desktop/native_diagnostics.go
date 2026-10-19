package main

import (
	"encoding/json"
	"errors"
	"net/url"
	"reflect"
	"strings"
	"unicode"

	"github.com/egoist/mygo/ui"
)

// Only these schema-1 fields can reach review, clipboard or export.
type diagnosticsReport struct {
	SchemaVersion uint64 `json:"schema_version"`
	Versions      struct {
		Core          string `json:"core"`
		RelayProtocol uint64 `json:"relay_protocol"`
	} `json:"versions"`
	ThisDevice struct {
		OS          string `json:"os"`
		IsRunner    bool   `json:"is_runner"`
		HasIdentity bool   `json:"has_identity"`
	} `json:"this_device"`
	Home struct {
		Exists bool `json:"exists"`
	} `json:"home"`
	Port struct {
		Free bool `json:"free"`
	} `json:"port"`
	Relay struct {
		Configured bool    `json:"configured"`
		Host       *string `json:"host"`
		Reachable  *bool   `json:"reachable"`
		Reason     string  `json:"reason"`
		HTTPStatus *uint64 `json:"http_status"`
		Protocol   *uint64 `json:"protocol"`
	} `json:"relay"`
	Providers struct {
		BuiltIn          []diagnosticsProvider `json:"built_in"`
		CustomConfigured uint64                `json:"custom_configured"`
		Health           string                `json:"health"`
	} `json:"providers"`
	Plugins struct {
		Basis   string              `json:"basis"`
		Entries []diagnosticsPlugin `json:"entries"`
	} `json:"plugins"`
	MCPJSON struct {
		Servers   uint64 `json:"servers"`
		Problems  uint64 `json:"problems"`
		FileError bool   `json:"file_error"`
	} `json:"mcp_json"`
	Runners []diagnosticsRunner `json:"runners"`
}
type diagnosticsProvider struct {
	Kind       string `json:"kind"`
	Configured bool   `json:"configured"`
}
type diagnosticsPlugin struct {
	Slot  uint64 `json:"slot"`
	State string `json:"state"`
}
type diagnosticsRunner struct {
	IsThisDevice bool    `json:"is_this_device"`
	OS           string  `json:"os"`
	Presence     string  `json:"presence"`
	Version      *string `json:"version"`
}

var errDiagnosticsInvalid = errors.New("Diagnostics report is invalid.")
var errDiagnosticsSchema = errors.New("This diagnostics schema is unsupported. Update Beans to review it.")

// encoding/json accepts missing and null scalar fields. Reject both for all
// required fields, while retaining the contract's nullable relay/Runner fields.
func diagnosticsRequired(raw json.RawMessage, typ reflect.Type) bool {
	if typ.Kind() == reflect.Pointer {
		return true
	}
	if len(raw) == 0 || strings.TrimSpace(string(raw)) == "null" {
		return false
	}
	switch typ.Kind() {
	case reflect.Struct:
		var fields map[string]json.RawMessage
		if json.Unmarshal(raw, &fields) != nil || fields == nil {
			return false
		}
		for i := 0; i < typ.NumField(); i++ {
			field := typ.Field(i)
			if !diagnosticsRequired(fields[field.Tag.Get("json")], field.Type) {
				return false
			}
		}
	case reflect.Slice:
		var entries []json.RawMessage
		if json.Unmarshal(raw, &entries) != nil {
			return false
		}
		for _, entry := range entries {
			if !diagnosticsRequired(entry, typ.Elem()) {
				return false
			}
		}
	}
	return true
}
func diagnosticsOneOf(value string, allowed ...string) bool {
	for _, item := range allowed {
		if value == item {
			return true
		}
	}
	return false
}
func diagnosticsRelease(value string) bool {
	parts := strings.Split(value, ".")
	if len(parts) != 3 {
		return false
	}
	for _, part := range parts {
		if len(part) == 0 || len(part) > 10 {
			return false
		}
		for _, ch := range part {
			if ch < '0' || ch > '9' {
				return false
			}
		}
	}
	return true
}
func diagnosticsHost(host string) bool {
	for _, ch := range host {
		if unicode.IsSpace(ch) || unicode.IsControl(ch) {
			return false
		}
	}
	u, err := url.Parse("https://" + host)
	return err == nil && u.Hostname() != "" && u.User == nil && u.Path == "" && u.RawQuery == "" && !u.ForceQuery && u.Fragment == "" && !strings.ContainsAny(host, "/?#\\") && u.String() == "https://"+host
}
func readDiagnostics(raw json.RawMessage) (string, *diagnosticsReport, error) {
	var schema struct {
		Version *uint64 `json:"schema_version"`
	}
	if json.Unmarshal(raw, &schema) != nil || schema.Version == nil {
		return "", nil, errDiagnosticsInvalid
	}
	if *schema.Version != 1 {
		return "", nil, errDiagnosticsSchema
	}
	var r diagnosticsReport
	if !diagnosticsRequired(raw, reflect.TypeOf(r)) || json.Unmarshal(raw, &r) != nil {
		return "", nil, errDiagnosticsInvalid
	}
	if !diagnosticsRelease(r.Versions.Core) || !diagnosticsOneOf(r.ThisDevice.OS, "macos", "linux", "windows", "ios", "ipados", "android", "unknown") || !diagnosticsOneOf(r.Relay.Reason, "none", "not_configured", "invalid_url", "unreachable", "update_required", "http_error") || r.Providers.Health != "not_checked" || r.Plugins.Basis != "cached_setup_state" {
		return "", nil, errDiagnosticsInvalid
	}
	if r.Relay.Host != nil && !diagnosticsHost(*r.Relay.Host) {
		return "", nil, errDiagnosticsInvalid
	}
	for _, p := range r.Providers.BuiltIn {
		if !diagnosticsOneOf(p.Kind, "deepseek", "anthropic", "opencode", "opencode-go", "chatgpt", "grok") {
			return "", nil, errDiagnosticsInvalid
		}
	}
	for _, p := range r.Plugins.Entries {
		if !diagnosticsOneOf(p.State, "ready", "needs_setup", "needs_auth", "connecting", "error", "unknown") {
			return "", nil, errDiagnosticsInvalid
		}
	}
	for _, runner := range r.Runners {
		if !diagnosticsOneOf(runner.OS, "macos", "linux", "windows") || !diagnosticsOneOf(runner.Presence, "online", "offline") || (runner.Version != nil && !diagnosticsRelease(*runner.Version)) {
			return "", nil, errDiagnosticsInvalid
		}
	}
	data, err := json.MarshalIndent(r, "", "  ")
	if err != nil {
		return "", nil, errDiagnosticsInvalid
	}
	return string(data), &r, nil
}

// All methods and injected completions are owned by the host's ordered main queue.
// The host additionally fences requests by admitted account and socket authority.
type nativeDiagnostics struct {
	request                       func(string, json.RawMessage, func(json.RawMessage, error))
	copyReport                    func(string) error
	exportReport                  func(string, func(error))
	invalidate                    func()
	generation                    uint64
	loading, exporting, done      bool
	report, disclosure, errorText string
}

func newNativeDiagnostics(request func(string, json.RawMessage, func(json.RawMessage, error)), copyReport func(string) error, exportReport func(string, func(error)), invalidate func()) *nativeDiagnostics {
	return &nativeDiagnostics{request: request, copyReport: copyReport, exportReport: exportReport, invalidate: invalidate}
}
func (d *nativeDiagnostics) changed() {
	if d.invalidate != nil {
		d.invalidate()
	}
}
func (d *nativeDiagnostics) Reset() {
	d.generation++
	d.loading, d.exporting, d.done = false, false, false
	d.report, d.disclosure, d.errorText = "", "", ""
	d.changed()
}
func (d *nativeDiagnostics) HasIntent() bool { return d.exporting }
func (d *nativeDiagnostics) Done() bool      { return d.done }
func (d *nativeDiagnostics) generate() {
	if d.loading || d.exporting || d.done {
		return
	}
	d.generation++
	generation := d.generation
	d.loading = true
	d.report, d.disclosure, d.errorText = "", "", ""
	d.changed()
	d.request("diagnostics.report", json.RawMessage(`{}`), func(raw json.RawMessage, err error) {
		if generation != d.generation || !d.loading {
			return
		}
		d.loading = false
		if err != nil {
			d.errorText = "Diagnostics could not be generated. This core may not support diagnostics.report."
		} else {
			report, r, readErr := readDiagnostics(raw)
			if readErr != nil {
				d.errorText = readErr.Error()
			} else {
				d.report = report
				host := "Not configured"
				if r.Relay.Host != nil {
					host = *r.Relay.Host
				}
				d.disclosure = "Shared relay host and explicit port: " + host
			}
		}
		d.changed()
	})
}
func (d *nativeDiagnostics) canShare() bool {
	return d.report != "" && !d.loading && !d.exporting && !d.done
}
func (d *nativeDiagnostics) copy() {
	if !d.canShare() {
		return
	}
	d.errorText = ""
	if d.copyReport(d.report) != nil {
		d.errorText = "Diagnostics report could not be copied."
	}
	d.changed()
}
func (d *nativeDiagnostics) export() {
	if !d.canShare() {
		return
	}
	d.exporting = true
	d.errorText = ""
	generation := d.generation
	d.changed()
	d.exportReport(d.report, func(err error) {
		if generation != d.generation || !d.exporting {
			return
		}
		d.exporting = false
		if err != nil {
			d.errorText = "Diagnostics report could not be exported."
		}
		d.changed()
	})
}
func (d *nativeDiagnostics) View(c *ui.Context) {
	ui.Text(c, "Diagnostics").Bold().FontSize(20)
	ui.Text(c, "Review before sharing. Relay host and explicit port are intentional disclosures. Provider flags show configuration, not working authentication or reachability. Plugin slots show anonymous cached setup state, not live health. Runner presence is not admission readiness. A busy port does not prove service failure. Generation may check selected public relay health; no provider probes or automatic upload.")
	label := "Generate"
	if d.report != "" {
		label = "Refresh"
	}
	ui.Row(c).Gap(8).Children(func() {
		if ui.Button(c, label).Disabled(d.loading || d.exporting || d.done).Clicked() {
			d.generate()
		}
		if ui.Button(c, "Copy Report").Disabled(!d.canShare()).Clicked() {
			d.copy()
		}
		if ui.Button(c, "Export Report").Disabled(!d.canShare()).Clicked() {
			d.export()
		}
		if ui.Button(c, "Close").Disabled(d.exporting).Clicked() {
			d.generation++
			d.loading = false
			d.done = true
			d.report, d.disclosure = "", ""
			d.changed()
		}
	})
	if d.loading {
		ui.Text(c, "Generating…")
	}
	if d.exporting {
		ui.Text(c, "Exporting…")
	}
	if d.errorText != "" {
		ui.Text(c, d.errorText)
	}
	if d.report != "" {
		ui.Text(c, d.disclosure).Selectable()
		ui.Text(c, "Review").Bold()
		ui.Scroll(c).Grow(1).Children(func() { ui.Text(c.Key("diagnostics-review"), d.report).Selectable() })
	}
}
