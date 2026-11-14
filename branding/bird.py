#!/usr/bin/env python3
"""The DesOS bird: one drawing, rendered for everything that shows it.

`bird.svg` is the master. Its single path (moves, lines, cubic curves, closes;
filled even-odd) is read here and drawn with cairo, so no SVG renderer is needed.
Run this after changing the drawing; it rewrites what the desktop embeds:

    compositor/assets/wordmark.la   the bird beside "DEs-UI", for the boot splash,
                                    the compositor's splash and the screensaver
    branding/icons/des-ui-<n>.png   the bird as an icon, at the usual sizes
"""
import re, sys
from pathlib import Path
import cairo

HERE = Path(__file__).resolve().parent
ROOT = HERE.parent
VIEW = 1000.0
# The drawing's extent inside its 1000 x 1000 box, so it can be placed by its ink.
INK = (68.0, 0.0, 932.0, 1000.0)


def commands(svg_text=None):
    text = svg_text if svg_text is not None else (HERE / "bird.svg").read_text()
    d = re.search(r'\sd="([^"]+)"', text).group(1)
    for op, args in re.findall(r"([MLCZ])([^MLCZ]*)", d):
        yield op, [float(v) for v in args.replace(",", " ").split()]


def trace(ctx, x, y, size):
    """Adds the bird to `ctx`'s path, its box `size` square with its corner at x, y."""
    k = size / VIEW
    for op, v in commands():
        p = [(x + v[i] * k, y + v[i + 1] * k) for i in range(0, len(v), 2)]
        if op == "M": ctx.move_to(*p[0])
        elif op == "L": ctx.line_to(*p[0])
        elif op == "C": ctx.curve_to(*p[0], *p[1], *p[2])
        else: ctx.close_path()


def draw(ctx, x, y, size, rgba=(1, 1, 1, 1)):
    ctx.save()
    ctx.set_fill_rule(cairo.FILL_RULE_EVEN_ODD)
    trace(ctx, x, y, size)
    ctx.set_source_rgba(*rgba)
    ctx.fill()
    ctx.restore()


def coverage(width, height, x, y, size):
    """The bird's coverage, 0-255 a pixel, row by row."""
    surface = cairo.ImageSurface(cairo.FORMAT_A8, width, height)
    draw(cairo.Context(surface), x, y, size)
    stride = surface.get_stride()
    data = bytes(surface.get_data())
    return [data[r * stride:r * stride + width] for r in range(height)]


def icon(size, out):
    surface = cairo.ImageSurface(cairo.FORMAT_ARGB32, size, size)
    pad = size * 0.04
    draw(cairo.Context(surface), pad, pad, size - 2 * pad, (0.949, 0.965, 0.973, 1))
    surface.write_to_png(str(out))


def lockup(out):
    """The bird beside the desktop's name: two bytes a pixel, how much is white (the
    bird and "DEs") and how much takes the accent ("-UI")."""
    name_w, name_h = 642, 126
    name = (HERE / "name.la").read_bytes()
    assert len(name) == name_w * name_h * 2
    # Where the hyphen starts: from there on, the name is in the accent.
    accent_from = min(x for y in range(name_h) for x in range(name_w) if name[(y * name_w + x) * 2 + 1] > 0)
    # Nine strips of whole rows, and even sides, since it is shown at half size.
    bird_h = 198
    ink_w = round((INK[2] - INK[0]) / VIEW * bird_h)
    gap = 35
    width, height = ink_w + gap + name_w, bird_h
    bird = coverage(width, height, -INK[0] / VIEW * bird_h, 0, bird_h)
    top = (height - name_h) // 2
    data = bytearray(width * height * 2)
    for y in range(height):
        for x in range(width):
            white = bird[y][x]
            accent = 0
            nx, ny = x - ink_w - gap, y - top
            if 0 <= nx < name_w and 0 <= ny < name_h:
                letter, dash = name[(ny * name_w + nx) * 2], name[(ny * name_w + nx) * 2 + 1]
                if nx >= accent_from - 2: accent = min(255, letter + dash)
                else: white = min(255, white + letter); accent = dash
            i = (y * width + x) * 2
            data[i], data[i + 1] = white, accent
    out.write_bytes(bytes(data))
    return width, height


if __name__ == "__main__":
    size = lockup(ROOT / "compositor/assets/wordmark.la")
    (HERE / "icons").mkdir(exist_ok=True)
    for n in (16, 24, 32, 48, 64, 128, 256, 512):
        icon(n, HERE / "icons" / f"des-ui-{n}.png")
    print(f"wordmark.la {size[0]}x{size[1]}; icons written")
