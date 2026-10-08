package main

import "context"

// Worker callers never inspect main-thread draft maps. The queue orders this
// admission snapshot with edits, completions and account transitions.
func nativeQuitAdmissionAsync(ctx context.Context) error {
	result := make(chan error, 1)
	postMain(func() { result <- nativeQuitAdmission() })
	select {
	case err := <-result:
		return err
	case <-ctx.Done():
		return ctx.Err()
	}
}
