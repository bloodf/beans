package main

import (
	"context"
	"net/http"
	"net/http/httptest"
	"testing"
)

func TestVerifyUpdateRelayFreshFormat(t *testing.T) {
	tests := []struct {
		name string
		body string
		wantOK bool
	}{
		{"fresh relay", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, true},
		{"future health extension", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1,"extension":{"enabled":true}}`, true},
		{"missing format", `{"ok":true,"service":"beans-relay","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"wrong format", `{"ok":true,"service":"beans-relay","format":"beans-v1","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"duplicate format", `{"ok":true,"service":"beans-relay","format":"beans-v1","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"old protocol", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":4,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"lowered floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":4,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"missing floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
		{"unsupported floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":6,"min_protocol":6,"min_roster_protocol":6,"memory_config_version":1}`, false},
		{"lowered roster floor", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":4,"memory_config_version":1}`, false},
		{"missing memory capability", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5}`, false},
		{"unsupported memory capability", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":5,"min_protocol":5,"min_roster_protocol":5,"memory_config_version":2}`, false},
		{"malformed protocol", `{"ok":true,"service":"beans-relay","format":"beans-v2","protocol":"5","min_protocol":5,"min_roster_protocol":5,"memory_config_version":1}`, false},
	}
	for _, test := range tests {
		t.Run(test.name, func(t *testing.T) {
			server := httptest.NewServer(http.HandlerFunc(func(w http.ResponseWriter, r *http.Request) {
				if r.URL.Path != "/v1/health" || r.Header.Get("Beans-Protocol") != "5" || r.Header.Get("Beans-Format") != "beans-v2" {
					w.WriteHeader(http.StatusUpgradeRequired)
					return
				}
				w.Header().Set("Content-Type", "application/json")
				_, _ = w.Write([]byte(test.body))
			}))
			defer server.Close()
			err := verifyUpdateRelay(context.Background(), server.URL, 5)
			if (err == nil) != test.wantOK {
				t.Fatalf("verifyUpdateRelay() = %v, want success %v", err, test.wantOK)
			}
		})
	}
}
