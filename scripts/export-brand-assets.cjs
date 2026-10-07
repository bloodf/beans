// Run with @resvg/resvg-js 2.6.2 available through NODE_PATH; see design/brand/README.md.
const { Resvg } = require('@resvg/resvg-js');
const { readFileSync, writeFileSync, copyFileSync, mkdtempSync, rmSync } = require('node:fs');
const { resolve, join } = require('node:path');
const { tmpdir } = require('node:os');
const { execFileSync } = require('node:child_process');

const root = resolve(__dirname, '..');
const brand = resolve(root, 'design/brand');
const scratch = mkdtempSync(join(tmpdir(), 'beans-icons-'));

function render(source, destination, width, opaque = false) {
  const svg = readFileSync(join(brand, source), 'utf8');
  const png = new Resvg(svg, { fitTo: { mode: 'width', value: width } }).render().asPng();
  const target = resolve(root, destination);
  writeFileSync(target, png);
  if (opaque) {
    execFileSync('python3', ['-c',
      'from PIL import Image; import sys; p=sys.argv[1]; im=Image.open(p); assert im.getchannel("A").getextrema()==(255,255); im.convert("RGB").save(p)', target]);
  }
}

try {
  for (const dev of [false, true]) {
    const suffix = dev ? '-dev' : '';
    const icon = `beans-app-icon${suffix}.svg`;
    render(icon, `mobile/assets/icon${suffix}.png`, 1024, true);
    render(icon, `mobile/assets/favicon${suffix}.png`, 64, true);
    render('beans-mark-square.svg', `mobile/assets/splash-icon${suffix}.png`, 1024);
    render(icon, `desktop/assets/icon${suffix}.png`, 512, true);
    copyFileSync(resolve(root, `desktop/assets/icon${suffix}.png`), resolve(root, `desktop/src/ui/images/icon${suffix}.png`));
    render(dev ? 'beans-adaptive-icon.svg' : 'beans-mark-square.svg', `desktop/assets/tray${suffix}.png`, 64);
    const iconset = join(scratch, `Beans${suffix}.iconset`);
    require('node:fs').mkdirSync(iconset);
    for (const size of [16, 32, 128, 256, 512]) {
      for (const scale of [1, 2]) {
        render(icon, join(iconset, `icon_${size}x${size}${scale === 2 ? '@2x' : ''}.png`), size * scale);
      }
    }
    execFileSync('iconutil', ['-c', 'icns', iconset, '-o', resolve(root, `macos/Resources/Beans${suffix}.icns`)]);
  }
  render('beans-adaptive-icon.svg', 'mobile/assets/adaptive-icon.png', 1024);
  render('beans-adaptive-icon-dev.svg', 'mobile/assets/adaptive-icon-dev.png', 1024);
  render('beans-app-icon.svg', 'web/public/icon.png', 256, true);
  render('beans-app-icon.svg', 'web/public/favicon.png', 64, true);
  render('beans-mark-square.svg', 'web/public/brand/beans-mark.png', 1254);
  render('beans-logo.svg', 'web/public/brand/beans-logo.png', 2161);
  for (const [source, destination] of [
    ['beans-mark-square.svg', 'beans-mark.svg'],
    ['beans-logo.svg', 'beans-logo.svg'],
    ['beans-logo-light.svg', 'beans-logo-light.svg'],
  ]) copyFileSync(join(brand, source), resolve(root, 'web/public/brand', destination));
} finally {
  rmSync(scratch, { recursive: true, force: true });
}
console.log('Exported Beans and Beans Dev assets from SVG masters.');
