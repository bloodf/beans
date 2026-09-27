//go:build !linux

package main

import "context"

// StartResize is Linux's: elsewhere the window keeps borders of its own to resize it by.
func (Host) StartResize(ctx context.Context, edge string, x, y int) {}
