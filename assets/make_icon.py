#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""Generate the two048 plugin icon (SVG source, rendered to PNG via rsvg-convert).

Palette is copied verbatim from the plugin's `tile_color()` mapping in src/lib.rs,
so the icon is visually consistent with the in-game board.
"""
import os

# --- Minecraft named colours, exactly as mapped in src/lib.rs::tile_color ---
TILE = {
    2:    ("#AAAAAA", "Gray"),
    4:    ("#FFFFFF", "White"),
    8:    ("#FFAA00", "Gold"),
    16:   ("#FFFF55", "Yellow"),
    32:   ("#FF5555", "Red"),
    64:   ("#AA0000", "DarkRed"),
    128:  ("#55FFFF", "Aqua"),
    256:  ("#00AAAA", "DarkAqua"),
    512:  ("#5555FF", "Blue"),
    1024: ("#FF55FF", "LightPurple"),
    2048: ("#55FF55", "Green"),
    4096: ("#00AA00", "DarkGreen"),
}
GREEN_ACCENT = "#55FF55"   # status-line / button green
GOLD_ACCENT = "#FFAA00"    # "2048" title colour
PUMPKIN = "#FF8A3D"        # nod to the Pumpkin server

W = H = 512

# --- geometry ---
GRID_X, GRID_Y = 100.0, 134.0
GRID_SIZE = 312.0
GAP = 8.0
CELL = (GRID_SIZE - GAP * 3) / 4          # 72.0
RADIUS = 14.0

BOARD = [
    [2,    4,    8,    16],
    [32,   64,   128,  256],
    [512,  1024, 2048, 4096],
    [4,    8,    16,   32],
]

FONT_UI = "'Segoe UI Semibold', 'Segoe UI', 'DejaVu Sans', Arial, sans-serif"
FONT_MONO = "'Cascadia Mono', Consolas, 'DejaVu Sans Mono', monospace"


def lift(hex_color, amount=0.22):
    """Mix a colour toward white so every glyph stays legible on the dark badge."""
    r = int(hex_color[1:3], 16)
    g = int(hex_color[3:5], 16)
    b = int(hex_color[5:7], 16)
    r = int(r + (255 - r) * amount)
    g = int(g + (255 - g) * amount)
    b = int(b + (255 - b) * amount)
    return "#%02X%02X%02X" % (r, g, b)


def font_size(value):
    digits = len(str(value))
    return {1: 40, 2: 36, 3: 30, 4: 25}[digits]


def build_svg():
    o = []
    a = o.append

    a(f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" '
      f'viewBox="0 0 {W} {H}" role="img" aria-label="two048 plugin icon">')

    # ---------------- defs ----------------
    a('<defs>')
    a('<linearGradient id="badge" x1="0" y1="0" x2="1" y2="1">'
      '<stop offset="0" stop-color="#1A1A25"/>'
      '<stop offset="0.55" stop-color="#121219"/>'
      '<stop offset="1" stop-color="#0A0A10"/>'
      '</linearGradient>')
    a('<radialGradient id="warm" cx="0.20" cy="0.10" r="0.68">'
      f'<stop offset="0" stop-color="{PUMPKIN}" stop-opacity="0.17"/>'
      '<stop offset="1" stop-color="#FF8A3D" stop-opacity="0"/>'
      '</radialGradient>')
    a('<radialGradient id="cool" cx="0.88" cy="1" r="0.66">'
      f'<stop offset="0" stop-color="{GREEN_ACCENT}" stop-opacity="0.10"/>'
      '<stop offset="1" stop-color="#55FF55" stop-opacity="0"/>'
      '</radialGradient>')
    a('<filter id="glow" x="-70%" y="-70%" width="240%" height="240%">'
      '<feGaussianBlur stdDeviation="10" result="b"/>'
      '<feMerge><feMergeNode in="b"/><feMergeNode in="b"/>'
      '<feMergeNode in="SourceGraphic"/></feMerge>'
      '</filter>')
    a('</defs>')

    # ---------------- badge ----------------
    a(f'<rect x="0" y="0" width="{W}" height="{H}" rx="116" fill="url(#badge)"/>')
    a(f'<rect x="0" y="0" width="{W}" height="{H}" rx="116" fill="url(#warm)"/>')
    a(f'<rect x="0" y="0" width="{W}" height="{H}" rx="116" fill="url(#cool)"/>')
    a(f'<rect x="1.5" y="1.5" width="{W-3}" height="{H-3}" rx="114.5" fill="none" '
      'stroke="#FFFFFF" stroke-opacity="0.10" stroke-width="3"/>')

    # ---------------- prompt row: the chat-bar identity ----------------
    a(f'<rect x="62" y="72" width="7" height="50" rx="3.5" fill="{PUMPKIN}"/>')
    a(f'<text x="86" y="110" font-family="{FONT_MONO}" font-size="40" '
      'font-weight="700" letter-spacing="0">'
      f'<tspan fill="{GREEN_ACCENT}">&gt;</tspan>'
      f'<tspan fill="{GOLD_ACCENT}">&#160;/2048</tspan>'
      '</text>')
    # terminal caret
    a(f'<rect x="256" y="82" width="14" height="34" rx="2" fill="{GREEN_ACCENT}" '
      'fill-opacity="0.8"/>')

    # ---------------- board ----------------
    for r in range(4):
        for c in range(4):
            value = BOARD[r][c]
            color = TILE[value][0]
            x = GRID_X + c * (CELL + GAP)
            y = GRID_Y + r * (CELL + GAP)
            hero = value == 2048

            if hero:
                a(f'<rect x="{x-5}" y="{y-5}" width="{CELL+10}" height="{CELL+10}" '
                  f'rx="{RADIUS+3}" fill="{color}" fill-opacity="0.60" filter="url(#glow)"/>')

            a(f'<rect x="{x}" y="{y}" width="{CELL}" height="{CELL}" rx="{RADIUS}" '
              f'fill="{color}" fill-opacity="{0.38 if hero else 0.16}" '
              f'stroke="{color}" stroke-opacity="{1.0 if hero else 0.48}" '
              f'stroke-width="{2.4 if hero else 1.5}"/>')

            fs = font_size(value)
            fs = fs - 2 if hero else fs
            fill = "#FFFFFF" if hero else lift(color, 0.30 if value != 4 else 0.05)
            cx = x + CELL / 2
            baseline = y + CELL / 2 + fs * 0.35
            a(f'<text x="{cx:.1f}" y="{baseline:.1f}" text-anchor="middle" '
              f'font-family="{FONT_UI}" font-size="{fs}" font-weight="700" '
              f'fill="{fill}">{value}</text>')

    # ---------------- control row: the plugin's clickable keys ----------------
    a(f'<text x="{W/2}" y="486" text-anchor="middle" font-family="{FONT_MONO}" '
      f'font-size="25" font-weight="600" letter-spacing="2.5" '
      f'fill="{GREEN_ACCENT}" fill-opacity="0.72">'
      '&#8593;W&#160;&#160;&#8592;A&#160;&#160;&#8595;S&#160;&#160;&#8594;D</text>')

    a('</svg>')
    return "\n".join(o)


if __name__ == "__main__":
    out_dir = os.path.dirname(os.path.abspath(__file__))
    svg_path = os.path.join(out_dir, "icon.svg")
    with open(svg_path, "w", encoding="utf-8") as fh:
        fh.write(build_svg())
    print(svg_path)
