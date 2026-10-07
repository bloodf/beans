# Beans identity assets

The editable vector masters contain Bézier paths with flat fills, transparent backgrounds and an outlined wordmark. No font installation or embedded bitmap is required.

| Asset | Use |
| --- | --- |
| beans-mark.svg | Tight transparent coral mark |
| beans-mark-square.svg | Transparent square canvas, preserving website spacing |
| beans-logo.svg | Coral mark and dark outlined wordmark |
| beans-logo-light.svg | Coral mark and off-white outlined wordmark for dark surfaces |
| beans-wordmark.svg | Dark outlined wordmark |
| beans-mark-mono.svg | Single-color mark; inherits currentColor |
| beans-logo-mono.svg | Single-color full logo; inherits currentColor |
| beans-app-icon.svg | Opaque square coral icon with off-white mark |
| beans-app-icon-dev.svg | Near-black development icon with coral mark |
| beans-adaptive-icon.svg | Transparent off-white Android foreground |
| beans-adaptive-icon-dev.svg | Transparent coral Android development foreground |

Colors: coral `#F26744`, near-black `#181A18`, off-white `#F7F8F4`.

The vectors are clean Potrace traces of the generated identity artwork, with speckles suppressed and curve optimization. The source PNGs in this directory retain the original image-generation output. Platform PNG/ICNS exports come from the SVG masters. The website serves the square SVG mark. Published 1.0.14 binaries retain their immutable packaged assets.

## Export platform assets

On macOS, install the isolated renderer and run the export script. It requires Node.js, Python 3 with Pillow, and macOS `iconutil`. It writes production and development app icons, Android transparent adaptive foregrounds, splash images, desktop tray/onboarding images, Mac ICNS files and website PNG/SVG assets. iOS icons have opaque RGB pixels.

```sh
npm install --prefix temp/beans-brand-renderer --no-save --package-lock=false @resvg/resvg-js@2.6.2
NODE_PATH="$PWD/temp/beans-brand-renderer/node_modules" node scripts/export-brand-assets.cjs
```

`prompts.md` records the built-in image-generation prompts. `beans-play-feature.png` is the 1024 × 500 Play listing graphic.
