import {
  ACESFilmicToneMapping, AmbientLight, Box3, DirectionalLight, ExtrudeGeometry,
  Group, HemisphereLight, Mesh, MeshPhysicalMaterial, PerspectiveCamera,
  Scene, Vector3, WebGLRenderer,
} from 'three'
import { SVGLoader } from 'three/addons/loaders/SVGLoader.js'
import { createEnergyField } from './energy-field'

export type BeanSceneController = {
  setPaused: (paused: boolean) => void
  dispose: () => void
}

export async function createBeanScene({ host, signal, onUnavailable }: {
  host: HTMLElement
  signal: AbortSignal
  onUnavailable: () => void
}): Promise<BeanSceneController> {
  const response = await fetch('/brand/beans-mark.svg', { signal })
  if (!response.ok) throw new Error('Brand mark unavailable')
  const svg = await response.text()
  signal.throwIfAborted()
  const renderer = new WebGLRenderer({ alpha: true, antialias: true, powerPreference: 'low-power' })
  const scene = new Scene()
  const camera = new PerspectiveCamera(32, 1, 0.1, 30)
  camera.position.z = 7.8
  renderer.setPixelRatio(Math.min(devicePixelRatio, 1.5))
  renderer.setClearColor(0x000000, 0)
  renderer.toneMapping = ACESFilmicToneMapping
  renderer.toneMappingExposure = 0.85
  const canvas = renderer.domElement
  canvas.setAttribute('aria-hidden', 'true')
  const material = new MeshPhysicalMaterial({ color: '#F26744', metalness: 0.28, roughness: 0.27, clearcoat: 1, clearcoatRoughness: 0.2 })
  const sculpture = new Group()
  const energy = createEnergyField()
  const geometries: ExtrudeGeometry[] = []
  try {
    const data = new SVGLoader().parse(svg)
    for (const path of data.paths) {
      const geometry = new ExtrudeGeometry(path.toShapes(), { depth: 85, bevelEnabled: true, bevelThickness: 16, bevelSize: 12, bevelSegments: 5, curveSegments: 12 })
      geometries.push(geometry)
      sculpture.add(new Mesh(geometry, material))
    }
    const bounds = new Box3().setFromObject(sculpture)
    const center = bounds.getCenter(new Vector3())
    const dimensions = bounds.getSize(new Vector3())
    for (const mesh of sculpture.children) mesh.position.sub(center)
    sculpture.scale.setScalar(3.15 / Math.max(dimensions.x, dimensions.y))
    sculpture.scale.y *= -1
    scene.add(sculpture)
    scene.add(energy.mesh)
    scene.add(new AmbientLight('#fff5ee', 0.7))
    scene.add(new HemisphereLight('#ffffff', '#b84220', 1.6))
    const key = new DirectionalLight('#fff8ed', 3)
    key.position.set(-3, 5, 6)
    scene.add(key)
    const rim = new DirectionalLight('#ffffff', 2)
    rim.position.set(4, -1, -2)
    scene.add(rim)
    host.appendChild(canvas)
  } catch (error) {
    for (const geometry of geometries) geometry.dispose()
    material.dispose()
    energy.geometry.dispose()
    energy.material.dispose()
    renderer.dispose()
    throw error
  }

  let paused = false
  let visible = true
  let disposed = false
  let frame = 0
  let lastTime = 0
  let elapsed = 0
  let pointerX = 0
  let pointerY = 0
  let rotationX = -0.12
  let rotationY = -0.35

  function draw(time: number) {
    frame = 0
    if (disposed || !visible || document.hidden) return
    const delta = Math.min((time - lastTime) / 1000, 0.05)
    lastTime = time
    if (!paused) elapsed += delta
    if (!paused) {
      rotationX += (pointerY * 0.25 - 0.12 - rotationX) * 0.06
      rotationY += (pointerX * 0.65 - 0.35 + Math.sin(elapsed * 0.55) * 0.32 - rotationY) * 0.06
      sculpture.rotation.set(rotationX, rotationY, -0.1 + Math.sin(elapsed * 0.42) * 0.13)
      sculpture.position.y = Math.sin(elapsed * 1.1) * 0.12
      energy.material.uniforms.time.value = elapsed
      energy.material.uniforms.pointer.value.set(pointerX, pointerY)
    }
    renderer.render(scene, camera)
    if (!paused) frame = requestAnimationFrame(draw)
  }

  function schedule() {
    if (!frame && !disposed && visible && !document.hidden) {
      lastTime = performance.now()
      frame = requestAnimationFrame(draw)
    }
  }
  function resize() {
    const width = host.clientWidth
    const height = host.clientHeight
    if (!width || !height) return
    camera.aspect = width / height
    camera.updateProjectionMatrix()
    renderer.setSize(width, height)
    schedule()
  }
  function pointer(event: PointerEvent) {
    if (event.pointerType !== 'mouse') return
    const bounds = host.getBoundingClientRect()
    pointerX = (event.clientX - bounds.left) / bounds.width * 2 - 1
    pointerY = (event.clientY - bounds.top) / bounds.height * 2 - 1
  }
  function leave() { pointerX = 0; pointerY = 0 }
  function visibility() {
    if (document.hidden) { cancelAnimationFrame(frame); frame = 0 } else schedule()
  }
  function contextLost(event: Event) {
    event.preventDefault()
    onUnavailable()
    visible = false
    cancelAnimationFrame(frame)
    frame = 0
  }
  const observer = new IntersectionObserver(([entry]) => {
    visible = Boolean(entry?.isIntersecting)
    if (visible) schedule()
    else { cancelAnimationFrame(frame); frame = 0 }
  })
  const sizeObserver = new ResizeObserver(resize)
  observer.observe(host)
  sizeObserver.observe(host)
  host.addEventListener('pointermove', pointer)
  host.addEventListener('pointerleave', leave)
  canvas.addEventListener('webglcontextlost', contextLost)
  document.addEventListener('visibilitychange', visibility)
  resize()

  return {
    setPaused(value) { paused = value; schedule() },
    dispose() {
      disposed = true
      cancelAnimationFrame(frame)
      observer.disconnect()
      sizeObserver.disconnect()
      host.removeEventListener('pointermove', pointer)
      host.removeEventListener('pointerleave', leave)
      canvas.removeEventListener('webglcontextlost', contextLost)
      document.removeEventListener('visibilitychange', visibility)
      for (const geometry of geometries) geometry.dispose()
      material.dispose()
      energy.geometry.dispose()
      energy.material.dispose()
      renderer.dispose()
      renderer.forceContextLoss()
      canvas.remove()
    },
  }
}
