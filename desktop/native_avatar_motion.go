package main

import (
	"encoding/json"
	"errors"
)

// nativeAvatarMotion owns one portrait's geometry, not a clock or window.
// All shape, easing and idle calculations belong to the shared renderer.
type nativeAvatarMotion struct {
	runtime               *sharedRuntime
	target, current, from json.RawMessage
	key                   string
	motion                bool
	retarget              bool
	started               float64
	lastTime              float64
	hasTime               bool
	frame                 avatarFrame
}

func newNativeAvatarMotion(runtime *sharedRuntime) *nativeAvatarMotion {
	return &nativeAvatarMotion{runtime: runtime}
}

func (m *nativeAvatarMotion) SetTarget(botID string, look json.RawMessage, state string) error {
	if len(look) == 0 {
		look = json.RawMessage("null")
	}
	key := botID + ":" + string(look) + ":" + state
	if key == m.key {
		return nil
	}
	var result struct {
		Geometry json.RawMessage `json:"geometry"`
		Motion   bool            `json:"motion"`
	}
	if err := m.runtime.eval(`const g=botAvatarGeometry(input.id,input.look,input.state); return {geometry:g,motion:g.motion};`, map[string]any{"id": botID, "look": look, "state": state}, &result); err != nil {
		return err
	}
	m.target = result.Geometry
	m.motion = result.Motion
	m.key = key
	m.from = m.current
	m.retarget = len(m.current) > 0
	return nil
}

func (m *nativeAvatarMotion) Frame(nowMS float64, visible, reduceMotion bool) (avatarFrame, bool, error) {
	if len(m.target) == 0 {
		return avatarFrame{}, false, errors.New("avatar target is not set")
	}
	moving := visible && !reduceMotion && m.motion
	if !moving {
		if err := m.runtime.eval(`return avatarFrame(input.target,0,0);`, map[string]any{"target": m.target}, &m.frame); err != nil {
			return avatarFrame{}, false, err
		}
		m.current = m.target
		m.from = nil
		m.retarget = false
		m.hasTime = false
		return m.frame, false, nil
	}
	if m.hasTime && nowMS < m.lastTime {
		nowMS = m.lastTime
	}
	if m.retarget {
		m.started = nowMS
		m.retarget = false
	}
	var result struct {
		Geometry json.RawMessage `json:"geometry"`
		Frame    avatarFrame     `json:"frame"`
		Done     bool            `json:"done"`
	}
	err := m.runtime.eval(`let g=input.target,done=true; if(input.from){const p=avatarMorphProgress(input.elapsed,input.target);g=interpolateAvatarGeometry(input.from,input.target,p.t);done=p.done;} return {geometry:g,frame:avatarFrame(g,input.now,1),done};`, map[string]any{"target": m.target, "from": m.from, "elapsed": nowMS - m.started, "now": nowMS}, &result)
	if err != nil {
		return avatarFrame{}, false, err
	}
	m.current = result.Geometry
	m.frame = result.Frame
	m.lastTime = nowMS
	m.hasTime = true
	if result.Done {
		m.from = nil
	}
	return m.frame, true, nil
}

func (m *nativeAvatarMotion) Reset() { *m = nativeAvatarMotion{runtime: m.runtime} }
