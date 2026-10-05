# Orochi desktop icon

A flat graphite tile with a simple white Yamata no Orochi emblem, eight distinct heads, red eyes, and one shared looping body. Generated with the built-in image generation tool; the transparent PNG is preserved as `source.png`.

The app bundle uses the assets in this directory through `../../tauri.conf.json`. The previous icons remain in the parent directory.

Regenerate the PNG, Windows ICO and macOS ICNS exports from the repository root with Python and Pillow:

```sh
python3 scripts/desktop-icons.py
```

## Generation prompt

Use case: logo-brand.
Asset type: production desktop application icon for Orochi, an AI coding agent orchestrator.
Create one exceptionally polished, cool and confident desktop app icon. Square 1024x1024 canvas with actual transparent background outside a centered rounded-square charcoal-black tile that occupies 88% of the canvas, standard macOS-like continuous corners. Front-facing orthographic, no perspective.
Subject: Yamata no Orochi, the Japanese eight-headed serpent. A compact white flat-vector emblem, with four distinct serpent heads on each side fanning upward from eight curving necks into one shared looping body. Tiny vermilion eye marks.
Style: very simple flat logo, like the user's supplied black icon with a clean white abstract mark. Near-black rounded square, crisp white shapes and restrained vermilion accents. Smooth clean curves and generous negative space.
Constraints: exactly eight distinct heads, four per side. No metallic surfaces, 3D, bevel, shine, shadows, gradients, texture, scales, text, letters, or extra symbols. True transparent pixels outside rounded tile.
