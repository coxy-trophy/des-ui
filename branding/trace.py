"""Trace the white bird in a PNG into a smooth SVG path (cubic Béziers)."""
import sys, numpy as np, contourpy
from PIL import Image
from scipy import ndimage

src = sys.argv[1]; STEP = float(sys.argv[2]) if len(sys.argv) > 2 else 6
img = np.asarray(Image.open(src).convert("L"), dtype=float) / 255.0
img = ndimage.gaussian_filter(img, 1.2)
gen = contourpy.contour_generator(z=img, line_type="SeparateCode")
lines, _ = gen.lines(0.5)
lines = [l for l in lines if len(l) > 40]
ys = np.concatenate(lines)
x0, y0 = ys.min(0); x1, y1 = ys.max(0)
scale = 1000 / max(x1 - x0, y1 - y0)
ox = (1000 - (x1 - x0) * scale) / 2; oy = (1000 - (y1 - y0) * scale) / 2

def resample(p, step):
    p = np.vstack([p, p[:1]]) if not np.allclose(p[0], p[-1]) else p
    d = np.r_[0, np.cumsum(np.hypot(*np.diff(p, axis=0).T))]
    n = max(8, int(d[-1] / step))
    t = np.linspace(0, d[-1], n, endpoint=False)
    return np.c_[np.interp(t, d, p[:, 0]), np.interp(t, d, p[:, 1])]

def corners(p, limit=0.55):
    a = np.roll(p, 1, 0) - p; b = np.roll(p, -1, 0) - p
    cos = (a * b).sum(1) / (np.hypot(*a.T) * np.hypot(*b.T) + 1e-9)
    return cos > -limit  # sharper than ~123 degrees

def smooth(p, keep, passes=3):
    for _ in range(passes):
        q = (np.roll(p, 1, 0) + 2 * p + np.roll(p, -1, 0)) / 4
        p = np.where(keep[:, None], p, q)
    return p

paths = []
for l in lines:
    p = (l - [x0, y0]) * scale + [ox, oy]
    p = resample(p, STEP)
    k = corners(p)
    # keep a neighbourhood around each sharp tip
    k = k | np.roll(k, 1) | np.roll(k, -1)
    p = smooth(p, k)
    n = len(p)
    d = f"M{p[0,0]:.1f} {p[0,1]:.1f}"
    for i in range(n):
        a, b, c, e = p[(i - 1) % n], p[i], p[(i + 1) % n], p[(i + 2) % n]
        if k[i] or k[(i + 1) % n]:
            d += f"L{c[0]:.1f} {c[1]:.1f}"
        else:
            c1 = b + (c - a) / 6; c2 = c - (e - b) / 6
            d += f"C{c1[0]:.1f} {c1[1]:.1f} {c2[0]:.1f} {c2[1]:.1f} {c[0]:.1f} {c[1]:.1f}"
    paths.append(d + "Z")
    area = 0.5*abs(np.dot(p[:,0],np.roll(p[:,1],1))-np.dot(p[:,1],np.roll(p[:,0],1)))
    print(f"{len(p)} pts area {area:.0f} bbox {p.min(0).round()} {p.max(0).round()}", file=sys.stderr)
print("".join(paths))
