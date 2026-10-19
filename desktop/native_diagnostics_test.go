package main

import (
	"encoding/json"
	"errors"
	"reflect"
	"strings"
	"testing"

	"github.com/egoist/mygo/ui"
)

const diagnosticsFixture = `{"schema_version":1,"versions":{"core":"0.8.1","relay_protocol":5},"this_device":{"os":"linux","is_runner":true,"has_identity":false},"home":{"exists":true},"port":{"free":false},"relay":{"configured":true,"host":"[::1]:8443","reachable":false,"reason":"unreachable","http_status":null,"protocol":null},"providers":{"built_in":[{"kind":"chatgpt","configured":true}],"custom_configured":2,"health":"not_checked"},"plugins":{"basis":"cached_setup_state","entries":[{"slot":1,"state":"needs_auth"}]},"mcp_json":{"servers":2,"problems":1,"file_error":false},"runners":[{"is_this_device":true,"os":"linux","presence":"offline","version":null}]}`

func diagnosticsObject(t *testing.T, text string) map[string]any {
	t.Helper()
	var obj map[string]any
	if err := json.Unmarshal([]byte(text), &obj); err != nil {
		t.Fatal(err)
	}
	return obj
}
func TestNativeDiagnosticsMountedReviewShareAndRevocation(t *testing.T) {
	var complete func(json.RawMessage, error)
	var exportDone func(error)
	var copied, exported string
	calls := 0
	d := newNativeDiagnostics(func(method string, params json.RawMessage, done func(json.RawMessage, error)) {
		calls++
		if method != "diagnostics.report" || string(params) != "{}" {
			t.Fatalf("unexpected request %s %s", method, params)
		}
		complete = done
	}, func(text string) error { copied = text; return nil }, func(text string, done func(error)) { exported = text; exportDone = done }, func() {})
	tt := ui.NewTester(func(c *ui.Context) { ui.Column(c).Fill().Children(func() { d.View(c) }) }, 1100, 2400)
	if calls != 0 {
		t.Fatal("constructor/view requested without consent")
	}
	if err := tt.Click("Generate"); err != nil {
		t.Fatal(err)
	}
	_ = tt.Click("Copy Report")
	if copied != "" || !d.loading {
		t.Fatal("pending copy dispatched")
	}
	clean := diagnosticsObject(t, diagnosticsFixture)
	dirty := diagnosticsObject(t, diagnosticsFixture)
	dirty["credentials"] = "SECRET_SENTINEL"
	dirty["relay"].(map[string]any)["url"] = "SECRET_SENTINEL"
	dirty["providers"].(map[string]any)["built_in"].([]any)[0].(map[string]any)["token"] = "SECRET_SENTINEL"
	dirty["plugins"].(map[string]any)["entries"].([]any)[0].(map[string]any)["name"] = "SECRET_SENTINEL"
	dirty["runners"].([]any)[0].(map[string]any)["id"] = "SECRET_SENTINEL"
	raw, _ := json.Marshal(dirty)
	complete(raw, nil)
	tt.Frame()
	if strings.Contains(d.report, "SECRET_SENTINEL") || !reflect.DeepEqual(diagnosticsObject(t, d.report), clean) {
		t.Fatal("sanitized report differs from schema payload")
	}
	if !tt.HasText(d.report) || !tt.HasText("Shared relay host and explicit port: [::1]:8443") {
		t.Fatal("review does not render sanitized payload/disclosure")
	}
	if err := tt.Click("Copy Report"); err != nil {
		t.Fatal(err)
	}
	if copied != d.report {
		t.Fatal("copy differs from reviewed report")
	}
	if err := tt.Click("Export Report"); err != nil {
		t.Fatal(err)
	}
	if exported != copied || !d.HasIntent() {
		t.Fatal("export differs or does not own pending intent")
	}
	_ = tt.Click("Close")
	_ = tt.Click("Refresh")
	if d.Done() || calls != 1 || !d.HasIntent() {
		t.Fatal("pending export admitted close or refresh")
	}
	exportDone(errors.New("SECRET_SENTINEL"))
	tt.Frame()
	if d.HasIntent() || !tt.HasText("Diagnostics report could not be exported.") || strings.Contains(strings.Join(tt.Texts(), " "), "SECRET_SENTINEL") {
		t.Fatal("export failed unsafe")
	}
	if err := tt.Click("Refresh"); err != nil {
		t.Fatal(err)
	}
	late := complete
	if d.report != "" {
		t.Fatal("refresh retained shareable report")
	}
	d.Reset()
	late(json.RawMessage(diagnosticsFixture), nil)
	tt.Frame()
	if d.report != "" || d.loading || d.HasIntent() {
		t.Fatal("reset admitted late completion")
	}
	if err := tt.Click("Generate"); err != nil {
		t.Fatal(err)
	}
	complete(nil, errors.New("SECRET_SENTINEL"))
	tt.Frame()
	if !tt.HasText("Diagnostics could not be generated. This core may not support diagnostics.report.") || strings.Contains(strings.Join(tt.Texts(), " "), "SECRET_SENTINEL") {
		t.Fatal("request failure leaked error")
	}
	if err := tt.Click("Generate"); err != nil {
		t.Fatal(err)
	}
	complete(json.RawMessage(`{"schema_version":2}`), nil)
	tt.Frame()
	if !tt.HasText(errDiagnosticsSchema.Error()) {
		t.Fatal("unsupported schema not shown")
	}
	_ = tt.Click("Export Report")
	if d.HasIntent() {
		t.Fatal("invalid report allowed export")
	}
	if err := tt.Click("Close"); err != nil || !d.Done() {
		t.Fatal("explicit close failed", err)
	}
}

func TestNativeDiagnosticsStrictSchemaBoundary(t *testing.T) {
	cases := []struct {
		name   string
		change func(map[string]any)
	}{
		{"missing required boolean", func(o map[string]any) { delete(o["home"].(map[string]any), "exists") }},
		{"null required boolean", func(o map[string]any) { o["port"].(map[string]any)["free"] = nil }},
		{"null required array", func(o map[string]any) { o["runners"] = nil }},
		{"null entry", func(o map[string]any) { o["runners"] = []any{nil} }},
		{"wrong boolean type", func(o map[string]any) { o["home"].(map[string]any)["exists"] = "true" }},
		{"negative count", func(o map[string]any) { o["mcp_json"].(map[string]any)["servers"] = -1 }},
		{"fractional count", func(o map[string]any) { o["mcp_json"].(map[string]any)["servers"] = 1.5 }},
		{"private core version", func(o map[string]any) { o["versions"].(map[string]any)["core"] = "/private/path" }},
		{"private runner version", func(o map[string]any) { o["runners"].([]any)[0].(map[string]any)["version"] = "secret" }},
		{"phone runner", func(o map[string]any) { o["runners"].([]any)[0].(map[string]any)["os"] = "ios" }},
		{"unknown presence", func(o map[string]any) { o["runners"].([]any)[0].(map[string]any)["presence"] = "ready" }},
		{"unknown provider", func(o map[string]any) {
			o["providers"].(map[string]any)["built_in"].([]any)[0].(map[string]any)["kind"] = "secret"
		}},
		{"live health", func(o map[string]any) { o["providers"].(map[string]any)["health"] = "working" }},
		{"unknown plugin state", func(o map[string]any) {
			o["plugins"].(map[string]any)["entries"].([]any)[0].(map[string]any)["state"] = "secret"
		}},
		{"unknown plugin basis", func(o map[string]any) { o["plugins"].(map[string]any)["basis"] = "live" }},
		{"raw relay reason", func(o map[string]any) { o["relay"].(map[string]any)["reason"] = "secret" }},
	}
	for _, host := range []string{"user:secret@example.org", "example.org/private", "example.org?secret", "example.org#secret", "example.org\n", ""} {
		cases = append(cases, struct {
			name   string
			change func(map[string]any)
		}{"host " + host, func(o map[string]any) { o["relay"].(map[string]any)["host"] = host }})
	}
	for _, tc := range cases {
		t.Run(tc.name, func(t *testing.T) {
			o := diagnosticsObject(t, diagnosticsFixture)
			tc.change(o)
			raw, _ := json.Marshal(o)
			text, r, err := readDiagnostics(raw)
			if err != errDiagnosticsInvalid || text != "" || r != nil {
				t.Fatal("invalid report accepted", text, err)
			}
		})
	}
	o := diagnosticsObject(t, diagnosticsFixture)
	relay := o["relay"].(map[string]any)
	for _, key := range []string{"host", "reachable", "http_status", "protocol"} {
		delete(relay, key)
	}
	delete(o["runners"].([]any)[0].(map[string]any), "version")
	raw, _ := json.Marshal(o)
	text, _, err := readDiagnostics(raw)
	if err != nil {
		t.Fatal(err)
	}
	normalized := diagnosticsObject(t, text)
	for _, key := range []string{"host", "reachable", "http_status", "protocol"} {
		v, ok := normalized["relay"].(map[string]any)[key]
		if !ok || v != nil {
			t.Fatal("missing nullable normalization", key)
		}
	}
	if v, ok := normalized["runners"].([]any)[0].(map[string]any)["version"]; !ok || v != nil {
		t.Fatal("Runner version missing null")
	}
}
