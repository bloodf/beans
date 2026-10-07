package main

import (
	"os"
	"path/filepath"
	"runtime"

	"github.com/egoist/mygo"
)

// productionRelayURL has no bundled default. Configure a Beans relay explicitly.
const productionRelayURL = ""

// isDevelopment is Beans Dev: `mygo dev` and `go run` builds, which keep their account and CLI
// apart from the installed Beans's.
func isDevelopment() bool { return mygo.IsDev() }

// isMock runs the seeded demo instead of the CLI (`BEANS_MOCK=1`), for screenshots.
func isMock() bool { return os.Getenv("BEANS_MOCK") == "1" }

// appName is the name the user sees: "Beans", or "Beans Dev" for a development build.
func appName() string {
	if name := mygo.App.Name(); name != "" {
		return name
	}
	if isDevelopment() {
		return "Beans Dev"
	}
	return "Beans"
}

func defaultCLIPort() int {
	if isDevelopment() {
		return 4875
	}
	return 4874
}

func defaultCLIHome() string {
	home, _ := os.UserHomeDir()
	if isDevelopment() {
		return filepath.Join(home, ".beans-dev-v2")
	}
	return filepath.Join(home, ".beans-v2")
}

// cliCommand is how to start the CLI by hand, for the offline state and the Help note.
func cliCommand() string {
	if isDevelopment() {
		return "beans serve --home ~/.beans-dev-v2 --port 4875"
	}
	return "beans serve --home ~/.beans-v2 --port 4874"
}

// logDir holds cli.log and startup.log.
func logDir() string {
	if dir, err := mygo.App.Path(mygo.PathLogs); err == nil {
		return dir
	}
	return filepath.Join(os.TempDir(), appName())
}

// platformName is the page's name for this system: "windows", "linux", or "darwin".
func platformName() string { return runtime.GOOS }
