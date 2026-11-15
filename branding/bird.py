#!/usr/bin/env python3
"""The DesOS bird: one drawing, rendered for everything that shows it.

`bird.svg` is the master. Its single path (moves, lines, cubic curves, closes;
filled even-odd) is read here and drawn with cairo, so no SVG renderer is needed.
Run this after changing the drawing; it rewrites what the desktop embeds:

    compositor/assets/wordmark.la   the bird beside "DEs-UI", for the screensaver
    compositor/assets/bird.la       the bird alone, for the boot splash and the
                                    compositor's splash (the loading screens)
    branding/icons/des-ui-<n>.png   the bird as an icon, at the usual sizes
"""
import re, sys
from pathlib import Path
try:
    import cairo
except ImportError:  # the bird alone can still be made without it (`--mark`)
    cairo = None

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


def flat_coverage(width, height, x, y, size, samples=8):
    """`coverage` without cairo: the path flattened to polygons and filled even-odd,
    `samples` x `samples` points a pixel. Matches cairo to within a level or two."""
    k = size / VIEW
    rings, ring, at = [], [], (0.0, 0.0)
    for op, v in commands():
        p = [(x + v[i] * k, y + v[i + 1] * k) for i in range(0, len(v), 2)]
        if op == "M":
            if ring: rings.append(ring)
            ring, at = [p[0]], p[0]
        elif op == "L":
            ring.append(p[0]); at = p[0]
        elif op == "C":
            (x0, y0), (x1, y1), (x2, y2), (x3, y3) = at, p[0], p[1], p[2]
            for n in range(1, 33):
                t = n / 32; u = 1 - t
                ring.append((u*u*u*x0 + 3*u*u*t*x1 + 3*u*t*t*x2 + t*t*t*x3,
                             u*u*u*y0 + 3*u*u*t*y1 + 3*u*t*t*y2 + t*t*t*y3))
            at = p[2]
        else:
            if ring: rings.append(ring)
            ring = []
    if ring: rings.append(ring)
    edges = [(a, b) for r in rings for a, b in zip(r, r[1:] + r[:1]) if a[1] != b[1]]
    rows = []
    for row in range(height):
        hits = [0] * width
        for sy in range(samples):
            yy = row + (sy + 0.5) / samples
            xs = sorted(ax + (yy - ay) * (bx - ax) / (by - ay)
                        for (ax, ay), (bx, by) in edges if (ay <= yy) != (by <= yy))
            for left, right in zip(xs[0::2], xs[1::2]):
                for sx in range(samples):
                    # sample columns: px + (sx + 0.5) / samples inside [left, right)
                    off = (sx + 0.5) / samples
                    first = max(0, int(-(-(left - off) // 1)))
                    last = min(width - 1, int((right - off) // 1) if (right - off) % 1 else int(right - off) - 1)
                    for px in range(first, last + 1):
                        hits[px] += 1
        total = samples * samples
        rows.append(bytes(min(255, round(h * 255 / total)) for h in hits))
    return rows


MARK_HEIGHT = 270  # nine strips of 30 rows; shown at half size, 135 px tall


def mark(out, height=MARK_HEIGHT, fill=None):
    """The bird alone, in the wordmark's format (the accent byte is always 0), its
    ink filling the picture: what the loading screens show."""
    width = round((INK[2] - INK[0]) / VIEW * height)
    width += width % 2  # even, since it is shown at half size
    fill = fill or (coverage if cairo else flat_coverage)
    bird = fill(width, height, -INK[0] / VIEW * height, 0, height)
    out.write_bytes(bytes(b for row in bird for v in row for b in (v, 0)))
    return width, height


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
    print("bird.la %dx%d" % mark(ROOT / "compositor/assets/bird.la"))
    if "--mark" in sys.argv:
        sys.exit(0)
    size = lockup(ROOT / "compositor/assets/wordmark.la")
    (HERE / "icons").mkdir(exist_ok=True)
    for n in (16, 24, 32, 48, 64, 128, 256, 512):
        icon(n, HERE / "icons" / f"des-ui-{n}.png")
    print(f"wordmark.la {size[0]}x{size[1]}; icons written")
