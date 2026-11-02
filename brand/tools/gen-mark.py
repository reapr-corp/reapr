# Final Reapr mark, tile and lockup. Usage: python3 brand/tools/gen-mark.py brand
#
# Blade: the spine is an Euler spiral (curvature linear in arc length), so it is G2 by construction.
# Back and cutting edge are offsets of the spine by a smoothly tapering width, so they stay G2 and
# meet only at the tip. Snath: a straight bar with slightly rounded corners.
# Tile: superellipse |x|^n + |y|^n = 1, n = 5, sampled and fitted with Catmull-Rom cubic Beziers.
import math, os, sys

OUT = sys.argv[1]
n2 = lambda v: f"{round(v, 2):g}"

# ---------- geometry ----------
def spine(p0, th0, L, k0, k1, N=400):
    x, y = p0; th = th0; ds = L / N; pts = []
    for i in range(N + 1):
        s = i * ds; pts.append((x, y, th, s / L))
        k = k0 + (k1 - k0) * (s / L)
        thm = th + k * ds / 2
        x += math.cos(thm) * ds; y += math.sin(thm) * ds; th += k * ds
    return pts

def scythe(P):
    phi = math.radians(P["tilt"]); up = (math.sin(phi), -math.cos(phi))
    B, T = P["bottom"], (P["bottom"][0] + up[0] * P["len"], P["bottom"][1] + up[1] * P["len"])
    t0 = (-up[1], up[0])
    S = (T[0] - up[0] * P["drop"] + t0[0] * P["W"] / 2 * P.get("sx", 0),
         T[1] - up[1] * P["drop"] + t0[1] * P["W"] / 2 * P.get("sx", 0))
    th0 = phi - math.radians(P["lift"])
    sp = spine(S, th0, P["blade"], P["k0"], P["k1"])
    back, edge = [], []
    for x, y, th, u in sp:
        w = P["wmax"] * max(0.0, 1 - u) ** P["p"]
        nx, ny = math.sin(th), -math.cos(th)          # visual left of travel = outer side
        back.append((x + nx * w * P["a"], y + ny * w * P["a"]))
        edge.append((x - nx * w * (1 - P["a"]), y - ny * w * (1 - P["a"])))
    blade = back + edge[::-1][1:]
    # snath: rectangle B..T, width W, corner radius r (polygonised for area, exact arcs for the path)
    W, r = P["W"], P["W"] * P["round"]
    t = (-up[1], up[0])                                # perpendicular (right of up)
    return dict(B=B, T=T, up=up, t=t, W=W, r=r, blade=blade, back=back, edge=edge)

def snath_poly(g):
    B, T, t, W = g["B"], g["T"], g["t"], g["W"]; h = W / 2
    return [(B[0] - t[0] * h, B[1] - t[1] * h), (T[0] - t[0] * h, T[1] - t[1] * h),
            (T[0] + t[0] * h, T[1] + t[1] * h), (B[0] + t[0] * h, B[1] + t[1] * h)]

def inside(poly, x, y):
    c = False; j = len(poly) - 1
    for i in range(len(poly)):
        xi, yi = poly[i]; xj, yj = poly[j]
        if (yi > y) != (yj > y) and x < (xj - xi) * (y - yi) / (yj - yi) + xi: c = not c
        j = i
    return c

def measure(g, steps=220):
    polys = [g["blade"][::8] + [g["blade"][-1]], snath_poly(g)]
    xs = [p[0] for q in polys for p in q]; ys = [p[1] for q in polys for p in q]
    x0, x1, y0, y1 = min(xs), max(xs), min(ys), max(ys)
    sx = sy = cnt = 0
    for i in range(steps):
        for j in range(steps):
            x = x0 + (x1 - x0) * (i + .5) / steps; y = y0 + (y1 - y0) * (j + .5) / steps
            if any(inside(q, x, y) for q in polys): sx += x; sy += y; cnt += 1
    return (x0, y0, x1, y1), (sx / cnt, sy / cnt)

# ---------- path output ----------
def catmull(pts, closed=False, tf=None):
    tf = tf or (lambda p: p)
    P = [tf(p) for p in pts]; m = len(P)
    get = (lambda i: P[i % m]) if closed else (lambda i: P[max(0, min(m - 1, i))])
    segs = m if closed else m - 1
    d = []
    for i in range(segs):
        p0, p1, p2, p3 = get(i - 1), get(i), get(i + 1), get(i + 2)
        c1 = (p1[0] + (p2[0] - p0[0]) / 6, p1[1] + (p2[1] - p0[1]) / 6)
        c2 = (p2[0] - (p3[0] - p1[0]) / 6, p2[1] - (p3[1] - p1[1]) / 6)
        d.append(f"C{n2(c1[0])} {n2(c1[1])} {n2(c2[0])} {n2(c2[1])} {n2(p2[0])} {n2(p2[1])}")
    return P[0], " ".join(d)

def blade_d(g, tf, n=18):
    # sample densely toward the tip (u -> 1) so the taper keeps its point after the Bezier fit
    m = len(g["back"]) - 1
    idx = sorted({round(m * (1 - (1 - i / n) ** 1.8)) for i in range(n + 1)})
    b = [g["back"][i] for i in idx]; e = [g["edge"][i] for i in idx]
    s0, db = catmull(b, tf=tf); _, de = catmull(e[::-1], tf=tf)
    return f"M{n2(s0[0])} {n2(s0[1])} {db} {de} Z"

def snath_d(g, tf, sc):
    B, T, t, up, W, r = g["B"], g["T"], g["t"], g["up"], g["W"], g["r"]; h = W / 2
    P = lambda base, a, b: tf((base[0] + t[0] * a + up[0] * b, base[1] + t[1] * a + up[1] * b))
    R = n2(r * sc); f = lambda p: f"{n2(p[0])} {n2(p[1])}"
    return (f"M{f(P(B, -h, r))} L{f(P(T, -h, -r))} A{R} {R} 0 0 1 {f(P(T, -h + r, 0))} "
            f"L{f(P(T, h - r, 0))} A{R} {R} 0 0 1 {f(P(T, h, -r))} L{f(P(B, h, r))} "
            f"A{R} {R} 0 0 1 {f(P(B, h - r, 0))} L{f(P(B, -h + r, 0))} A{R} {R} 0 0 1 {f(P(B, -h, r))} Z")

def squircle_d(size, n=5, N=128, inset=0):
    a = size / 2 - inset; c = size / 2; pts = []
    for i in range(N):
        t = 2 * math.pi * i / N; ct, st = math.cos(t), math.sin(t)
        pts.append((c + a * math.copysign(abs(ct) ** (2 / n), ct), c + a * math.copysign(abs(st) ** (2 / n), st)))
    s0, d = catmull(pts, closed=True)
    return f"M{n2(s0[0])} {n2(s0[1])} {d} Z"

def place(g, box, pad, w_centroid=0.7):
    """Scale the scythe into a square box (size `box`) with padding `pad`, centred by visual weight."""
    (x0, y0, x1, y1), (cx, cy) = measure(g)
    sc = (box * (1 - 2 * pad)) / max(x1 - x0, y1 - y0)
    ox = w_centroid * cx + (1 - w_centroid) * (x0 + x1) / 2
    oy = w_centroid * cy + (1 - w_centroid) * (y0 + y1) / 2
    tf = lambda p: (box / 2 + (p[0] - ox) * sc, box / 2 + (p[1] - oy) * sc)
    bb = (tf((x0, y0)), tf((x1, y1)))
    return tf, sc, bb

# ---------- parameters (design space ~100 units) ----------
import json
FULL = dict(bottom=(0, 100), tilt=27, len=92, W=6.4, round=.28, drop=12, lift=12,
            blade=66, k0=1 / 80, k1=1 / 26, wmax=15, p=1.1, a=.42)
SMALL = dict(FULL, W=11, drop=4, sx=-1, lift=14, blade=56, k0=1 / 70, k1=1 / 22, wmax=25, p=.9, a=.45)
if os.environ.get("FULL"): FULL.update(json.loads(os.environ["FULL"]))
if os.environ.get("SMALL"): SMALL.update(json.loads(os.environ["SMALL"]))

def scythe_paths(P, box, pad, wc=0.0):
    g = scythe(P); tf, sc, bb = place(g, box, pad, wc)
    return snath_d(g, tf, sc), blade_d(g, tf), bb

def write(name, s):
    os.makedirs(os.path.dirname(os.path.join(OUT, name)), exist_ok=True)
    open(os.path.join(OUT, name), "w").write(s)

# mark.svg / mark-small.svg: standalone scythe, currentColor, 32-unit canvas
for P, name, pad in ((FULL, "mark.svg", .03), (SMALL, "mark-small.svg", .02)):
    s, b, bb = scythe_paths(P, 32, pad)
    write(name, f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32">\n  <title>Reapr</title>\n'
                f'  <g fill="currentColor">\n    <path d="{s}"/>\n    <path d="{b}"/>\n  </g>\n</svg>\n')
    print(name, "bbox", [tuple(round(v, 2) for v in p) for p in bb])

INK, BONE = "#0B0D0C", "#E8EAE6"
TILE = squircle_d(1024)

def icon(P, name, pad, wc=0.4):
    s, b, bb = scythe_paths(P, 1024, pad, wc)
    write(name, f'<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">\n  <title>Reapr</title>\n'
                f'  <path fill="{INK}" d="{TILE}"/>\n  <g fill="{BONE}">\n    <path d="{s}"/>\n    <path d="{b}"/>\n  </g>\n</svg>\n')
    print(name, "padding %", [round(v / 10.24, 1) for v in (bb[0][0], bb[0][1], 1024 - bb[1][0], 1024 - bb[1][1])])
    return s, b

s, b = icon(FULL, "icon.svg", .20)
icon(SMALL, "icon-small.svg", .16)

# icon-app.svg: app-icon finish (tonal gradient, top inner highlight, very soft shadow under the scythe)
write("icon-app.svg", f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <title>Reapr</title>
  <defs>
    <linearGradient id="tone" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#1A1D1B"/>
      <stop offset="1" stop-color="{INK}"/>
    </linearGradient>
    <linearGradient id="rim" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="#FFFFFF" stop-opacity=".14"/>
      <stop offset=".16" stop-color="#FFFFFF" stop-opacity="0"/>
    </linearGradient>
    <clipPath id="tile"><path d="{TILE}"/></clipPath>
    <filter id="shade" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="14"/>
    </filter>
  </defs>
  <path fill="url(#tone)" d="{TILE}"/>
  <path fill="none" stroke="url(#rim)" stroke-width="4" clip-path="url(#tile)" d="{TILE}"/>
  <g fill="#000" opacity=".35" filter="url(#shade)" transform="translate(0 12)">
    <path d="{s}"/>
    <path d="{b}"/>
  </g>
  <g fill="{BONE}">
    <path d="{s}"/>
    <path d="{b}"/>
  </g>
</svg>
''')

# lockup.svg: mark + wordmark, currentColor. The scythe spans from above the x-height to the descender.
wm = open(os.path.join(OUT, "wordmark.svg")).read()
wm_paths = "\n".join("    " + l.strip() for l in wm.splitlines() if l.strip().startswith(("<path", "<circle")))
g = scythe(FULL); (x0, y0, x1, y1), _ = measure(g)
H = 50; sc = H / (y1 - y0)                       # y from -8 to 42
tf = lambda p: ((p[0] - x0) * sc, -8 + (p[1] - y0) * sc)
mw = (x1 - x0) * sc; gap = 11
write("lockup.svg", f'''<svg xmlns="http://www.w3.org/2000/svg" viewBox="-8 -16 {n2(mw + gap + 118 + 16)} 66">
  <title>Reapr</title>
  <g fill="currentColor">
    <path d="{snath_d(g, tf, sc)}"/>
    <path d="{blade_d(g, tf)}"/>
  </g>
  <g transform="translate({n2(mw + gap)} 0)" fill="none" stroke="currentColor" stroke-width="3.2">
{wm_paths}
  </g>
</svg>
''')
print("lockup mark width", round(mw, 1))
