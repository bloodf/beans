package main

import (
    "net/http/httptest"
    "testing"
)

func TestDevelopmentProxyRejectsForeignCallerBeforeCredentialRead(t *testing.T) {
    t.Setenv("BEANS_HOME", t.TempDir()+"/absent")
    for _, test := range []struct { peer, host, origin string }{
        {"192.0.2.1:1234", "localhost:5178", "http://localhost:5178"},
        {"127.0.0.1:1234", "foreign.invalid:5178", "http://localhost:5178"},
        {"127.0.0.1:1234", "localhost:5178", "https://foreign.invalid"},
        {"127.0.0.1:1234", "localhost:5178", ""},
    } {
        request := httptest.NewRequest("GET", "http://localhost:5178/__beans_cli_ws", nil)
        request.RemoteAddr, request.Host = test.peer, test.host
        if test.origin != "" { request.Header.Set("Origin", test.origin) }
        response := httptest.NewRecorder()
        developmentCLIUpgrade(response, request)
        if response.Code != 403 { t.Fatalf("foreign caller reached credential admission: %d", response.Code) }
    }
}
