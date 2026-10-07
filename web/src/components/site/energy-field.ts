import { Mesh, PlaneGeometry, ShaderMaterial, Vector2 } from 'three'

/** Animated contours follow the bean's silhouette through the same bounded WebGL loop. */
export function createEnergyField() {
  const material = new ShaderMaterial({
    transparent: true,
    depthWrite: false,
    uniforms: {
      time: { value: 0 },
      pointer: { value: new Vector2() },
    },
    vertexShader: `
      varying vec2 uvPosition;
      void main() {
        uvPosition = uv;
        gl_Position = projectionMatrix * modelViewMatrix * vec4(position, 1.0);
      }
    `,
    fragmentShader: `
      precision highp float;
      varying vec2 uvPosition;
      uniform float time;
      uniform vec2 pointer;
      void main() {
        vec2 p = (uvPosition - 0.5) * 2.0 - pointer * 0.035;
        p.x *= 1.04;
        float angle = atan(p.y, p.x);
        float radius = length(p);
        float bend = sin(angle * 3.0 + time * 0.42) * 0.035
          + sin(angle * 5.0 - time * 0.28) * 0.015;
        float field = radius + bend;
        float ink = 0.0;
        float bloom = 0.0;
        for (int i = 0; i < 5; i++) {
          float lane = float(i);
          float ring = 0.38 + lane * 0.105;
          float distanceToLine = abs(field - ring);
          float flow = pow(max(0.0, sin(angle * 1.5 - time * 0.7 + lane * 1.8)), 5.0);
          ink += (1.0 - smoothstep(0.0015, 0.005, distanceToLine)) * (0.12 + flow * 0.75);
          bloom += exp(-distanceToLine * 85.0) * flow * 0.16;
        }
        float edge = 1.0 - smoothstep(0.82, 0.97, radius);
        vec3 coral = mix(vec3(0.95, 0.22, 0.08), vec3(1.0, 0.64, 0.33),
          sin(angle + time * 0.3) * 0.5 + 0.5);
        float alpha = clamp((ink + bloom) * edge, 0.0, 0.85);
        gl_FragColor = vec4(coral, alpha);
      }
    `,
  })
  const geometry = new PlaneGeometry(6.7, 6.7)
  const mesh = new Mesh(geometry, material)
  mesh.position.z = -1.4
  return { mesh, material, geometry }
}
