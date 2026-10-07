'use client'

import { useEffect, useRef, useState } from 'react'
import { useSiteMotion } from './motion'
import type { BeanSceneController } from './bean-webgl'

export function BeanScene() {
  const host = useRef<HTMLDivElement>(null)
  const controller = useRef<BeanSceneController | null>(null)
  const { paused, reduced } = useSiteMotion()
  const [ready, setReady] = useState(false)

  useEffect(() => {
    const element = host.current
    if (!element || reduced) return
    const abort = new AbortController()
    let disposed = false
    void import('./bean-webgl').then(({ createBeanScene }) => createBeanScene({ host: element, signal: abort.signal, onUnavailable: () => setReady(false) })).then((scene) => {
      if (disposed) { scene.dispose(); return }
      controller.current = scene
      setReady(true)
    }).catch(() => { if (!disposed) setReady(false) })
    return () => {
      disposed = true
      abort.abort()
      controller.current?.dispose()
      controller.current = null
      setReady(false)
    }
  }, [reduced])

  useEffect(() => { controller.current?.setPaused(paused) }, [paused, ready])

  return (
    <div className="hero-scene motion-island" aria-hidden="true" data-ready={ready && !reduced}>
      <div className="scene-orbit scene-orbit-one" />
      <div className="scene-orbit scene-orbit-two" />
      <div className="scene-floor" />
      <img src="/brand/beans-mark.svg" className="scene-fallback" alt="" width={1254} height={1254} fetchPriority="high" />
      <div ref={host} className="scene-canvas" />
    </div>
  )
}
