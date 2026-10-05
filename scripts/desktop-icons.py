#!/usr/bin/env python3
"""Export the desktop icon from its source PNG. Requires Pillow."""

from pathlib import Path

from PIL import Image


def main():
    directory = Path(__file__).resolve().parents[1] / "desktop/src-tauri/icons/orochi"
    with Image.open(directory / "source.png") as source:
        if source.width != source.height:
            raise ValueError("The icon source must be square")
        image = source.convert("RGBA")
        for filename, size in (
            ("icon.png", 1024),
            ("32x32.png", 32),
            ("128x128.png", 128),
            ("128x128@2x.png", 256),
        ):
            image.resize((size, size), Image.Resampling.LANCZOS).save(directory / filename)
        image.resize((256, 256), Image.Resampling.LANCZOS).save(
            directory / "icon.ico",
            sizes=[(size, size) for size in (16, 24, 32, 48, 64, 128, 256)],
        )
        image.resize((1024, 1024), Image.Resampling.LANCZOS).save(directory / "icon.icns")
    print(f"Exported desktop icons to {directory}")


if __name__ == "__main__":
    main()
