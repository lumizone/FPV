#!/usr/bin/env python3
# Regenerate the DMG background image. Run once whenever the brand
# colours or DMG window geometry change. Output: background.png (500x350)
# and background@2x.png (1000x700). Finder picks the @2x file on retina
# displays automatically when both sit in `.background/` of the mounted
# DMG (we copy both via the same --background flag — create-dmg/
# bundle_dmg.sh handles the @2x lookup).

from PIL import Image, ImageDraw

WIDTH = 500
HEIGHT = 350

# Soft lavender → pink gradient. Matches the FPV app icon's
# pastel palette and reads as "approachable consumer app" rather than
# the default white DMG. Tweak these two colours and re-render to
# rebrand the installer panel.
TOP = (245, 234, 250)      # lavender mist
BOTTOM = (252, 235, 240)   # blush

# Centre Y where the icons sit (matches `--icon … 110 150` and
# `--app-drop-link 380 150` in scripts/package.sh). Keep in sync if
# those positions change.
ICON_Y = 150
LEFT_X = 110
RIGHT_X = 380


def render(scale: int) -> Image.Image:
    w, h = WIDTH * scale, HEIGHT * scale
    img = Image.new("RGB", (w, h), TOP)
    for y in range(h):
        t = y / (h - 1)
        r = int(TOP[0] + (BOTTOM[0] - TOP[0]) * t)
        g = int(TOP[1] + (BOTTOM[1] - TOP[1]) * t)
        b = int(TOP[2] + (BOTTOM[2] - TOP[2]) * t)
        for x in range(w):
            img.putpixel((x, y), (r, g, b))
    # Pillow's per-pixel loop is slow; switch to a faster path that
    # paints horizontal lines instead.
    img = Image.new("RGB", (w, h), TOP)
    draw = ImageDraw.Draw(img)
    for y in range(h):
        t = y / (h - 1)
        r = int(TOP[0] + (BOTTOM[0] - TOP[0]) * t)
        g = int(TOP[1] + (BOTTOM[1] - TOP[1]) * t)
        b = int(TOP[2] + (BOTTOM[2] - TOP[2]) * t)
        draw.line([(0, y), (w, y)], fill=(r, g, b))

    # Chevron sitting between the two icon slots — purely decorative
    # but the visual rhyme with macOS Finder navigation arrows makes
    # the "drag right" intent unmistakable.
    cx = (LEFT_X + RIGHT_X) // 2 * scale
    cy = ICON_Y * scale
    arm = 22 * scale
    thickness = max(6 * scale, 1)
    arrow_colour = (140, 120, 175)  # muted purple, ~3:1 contrast on bg
    draw.line(
        [(cx - arm, cy - arm), (cx + arm, cy), (cx - arm, cy + arm)],
        fill=arrow_colour,
        width=thickness,
        joint="curve",
    )

    return img


def main() -> None:
    here = __file__.rsplit("/", 1)[0]
    render(1).save(f"{here}/background.png", "PNG")
    render(2).save(f"{here}/background@2x.png", "PNG")
    print("wrote background.png + background@2x.png")


if __name__ == "__main__":
    main()
