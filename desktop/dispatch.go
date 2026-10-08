package main

import (
	"github.com/egoist/mygo"
	"sync"
)

// Main-thread work is drained in arrival order, including reentrant posts.
var mainPosts struct {
	sync.Mutex
	queue   []func()
	running bool
}

var executeMain = mygo.RunOnMain

func postMain(fn func()) {
	mainPosts.Lock()
	mainPosts.queue = append(mainPosts.queue, fn)
	if mainPosts.running {
		mainPosts.Unlock()
		return
	}
	mainPosts.running = true
	mainPosts.Unlock()
	go func() {
		for {
			mainPosts.Lock()
			batch := mainPosts.queue
			mainPosts.queue = nil
			if len(batch) == 0 {
				mainPosts.running = false
				mainPosts.Unlock()
				return
			}
			mainPosts.Unlock()
			executeMain(func() {
				for _, fn := range batch {
					fn()
				}
			})
		}
	}()
}
