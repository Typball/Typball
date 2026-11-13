"""Typball のアイコン (SVG) を生成する。

タイプボール (活字が並んだ印字ヘッド) を簡略化したクロムの球を、ロックグラスのウイスキーに
入れた絵 (Typball = Typeball + Highball)。

    python design/icons/gen_icons.py      # SVG を作る
    python design/icons/export_icons.py   # ICO / ICNS / RGBA に書き出す

出力 (このフォルダ):
    app.svg              アプリのアイコン (Windows / Linux)。24px 以上
    app-macos.svg        macOS 用 (生成り地の角丸正方形に app.svg と同じ絵)。32px 以上
    app-16.svg           16〜20px 用 (16 の格子に合わせた簡略版)
    doc-{ext}.svg        関連付けアイコン。64px 以上
    doc-{ext}-small.svg  同 24〜48px 用 (単色の球 + 拡張子の帯)
    doc-{ext}-16.svg     同 16px 用 (文字なし)
"""

import math
import os
import random

OUT = os.path.dirname(os.path.abspath(__file__))
FONT = "'Courier New', Courier, monospace"
TILT = math.radians(24)  # 見下ろす角度。球の天面とグラスの楕円で揃える
SA, CA = math.sin(TILT), math.cos(TILT)


def fmt(v):
    return f"{v:.2f}".rstrip("0").rstrip(".")


def svg(size, defs, body):
    return (
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{size}" height="{size}" viewBox="0 0 {size} {size}">\n'
        f"<defs>{defs}\n</defs>\n  {body}\n</svg>\n"
    )


def write(name, text):
    with open(os.path.join(OUT, name), "w", encoding="utf-8", newline="\n") as f:
        f.write(text)


# ---------------------------------------------------------------- 球 (簡略版・クロム)

CHROME = {
    "body": ["#ffffff", "#e9ecf0", "#c2c8d0", "#8e96a2"],  # 中心 → 縁
    "cap": "#f2f4f7",
    "ink": "#2e343d",
}


def ball(pid, cx, cy, R, glyph="T", lines=True, glyph_scale=1.0, solid=None):
    """簡略版のタイプボール。正面の 1 字・緯線 2 本・天面のハブだけ。
    solid に色を渡すとグラデーション無しの単色 (小さな関連付けアイコン用)。
    (defs, body) を返す"""
    c = CHROME["body"]
    defs = f"""
  <radialGradient id="{pid}-g" cx="0.5" cy="0.5" r="0.5" fx="0.36" fy="0.3">
    <stop offset="0" stop-color="{c[0]}"/><stop offset="0.3" stop-color="{c[1]}"/>
    <stop offset="0.75" stop-color="{c[2]}"/><stop offset="1" stop-color="{c[3]}"/>
  </radialGradient>
  <clipPath id="{pid}-clip"><circle cx="{fmt(cx)}" cy="{fmt(cy)}" r="{fmt(R)}"/></clipPath>"""
    ink = CHROME["ink"]
    fill, cap, line_col, line_op, hub = f"url(#{pid}-g)", CHROME["cap"], ink, 0.22, ink
    if solid:
        fill, cap, line_col, line_op, hub = solid, "#ffffff", "#ffffff", 0.4, solid
    out = [f'<circle cx="{fmt(cx)}" cy="{fmt(cy)}" r="{fmt(R)}" fill="{fill}"/>']
    if lines:
        ls = []
        for lat in (8, -16):
            ph = math.radians(lat)
            rr = R * math.cos(ph)
            yc = cy - R * math.sin(ph) * CA
            ls.append(
                f'<ellipse cx="{fmt(cx)}" cy="{fmt(yc)}" rx="{fmt(rr)}" ry="{fmt(rr * SA)}" fill="none" '
                f'stroke="{line_col}" stroke-opacity="{line_op}" stroke-width="{fmt(R * 0.07)}"/>'
            )
        out.append(f'<g clip-path="url(#{pid}-clip)">' + "".join(ls) + "</g>")
    phc = math.radians(62)
    r = R * math.cos(phc)
    ycap = cy - R * math.sin(phc) * CA
    out.append(
        f'<ellipse cx="{fmt(cx)}" cy="{fmt(ycap)}" rx="{fmt(r)}" ry="{fmt(r * SA)}" fill="{cap}" '
        f'stroke="{ink}" stroke-opacity="0.18" stroke-width="{fmt(R * 0.02)}"/>'
    )
    out.append(
        f'<ellipse cx="{fmt(cx)}" cy="{fmt(ycap)}" rx="{fmt(r * 0.45)}" ry="{fmt(r * 0.45 * SA)}" fill="{hub}" fill-opacity="0.8"/>'
    )
    if glyph:
        fs = R * 0.95 * glyph_scale
        out.append(
            f'<text x="{fmt(cx)}" y="{fmt(cy + fs * 0.44)}" font-family="{FONT}" font-weight="bold" '
            f'font-size="{fmt(fs)}" text-anchor="middle" fill="{ink}">{glyph}</text>'
        )
    if solid:
        return defs, "\n  ".join(out)
    # 縁の照り返し
    out.append(
        f'<path d="M{fmt(cx + R * 0.94 * math.cos(0.3))},{fmt(cy + R * 0.94 * math.sin(0.3))} '
        f'A{fmt(R * 0.94)},{fmt(R * 0.94)} 0 0 1 {fmt(cx + R * 0.94 * math.cos(1.6))},{fmt(cy + R * 0.94 * math.sin(1.6))}" '
        f'fill="none" stroke="#ffffff" stroke-opacity="0.55" stroke-width="{fmt(R * 0.04)}" stroke-linecap="round"/>'
    )
    return defs, "\n  ".join(out)


# ---------------------------------------------------------------- グラス

WHISKY, SURFACE, LIQ_OPACITY = "#e9a43a", "#f7d58a", 0.55


def glass_in(pid, cx, top, bottom, rt, rb, R, level, bubbles, base=0.1, seed=5, glyph_scale=1.35):
    """ロックグラスに入った球。top/bottom は縁と底 (外側) の楕円の中心 y、rt/rb はその半幅。
    level は液面の高さ (内側の底からの割合)。球は底に置く。
    はみ出さない目安: 縁の奥 top - rt*SA と、接地影の下端 bottom + rb*SA*1.25 がキャンバスに収まること。
    (defs, body) を返す"""
    k = SA  # 楕円の縦横比
    ib = bottom - base * (bottom - top)  # 内側の底
    wall = rt * 0.04

    def half_w(y):  # 高さ y の外側の半幅
        return rb + (rt - rb) * (bottom - y) / (bottom - top)

    rib = half_w(ib)
    cy = ib - R * CA
    ly = ib - level * (ib - top)
    lw = half_w(ly) - wall

    def silhouette(y0, w0, y1, w1):
        # 上の楕円の奥側 + 側面 + 下の楕円の手前側
        return (
            f"M{fmt(cx - w0)},{fmt(y0)} L{fmt(cx - w1)},{fmt(y1)} "
            f"A{fmt(w1)},{fmt(w1 * k)} 0 0 0 {fmt(cx + w1)},{fmt(y1)} L{fmt(cx + w0)},{fmt(y0)} "
            f"A{fmt(w0)},{fmt(w0 * k)} 0 0 0 {fmt(cx - w0)},{fmt(y0)} Z"
        )

    body_path = silhouette(top, rt, bottom, rb)
    liq_path = silhouette(ly, lw, ib, rib - wall)
    gs = "#7d8794"
    ri = rt - wall * 1.5

    def rim(back):
        # 縁の楕円を奥 (球より先) と手前 (球より後) に分けて描く
        a, b = (cx + rt, cx - rt) if back else (cx - rt, cx + rt)
        ai, bi = (cx + ri, cx - ri) if back else (cx - ri, cx + ri)
        return (
            f'<path d="M{fmt(a)},{fmt(top)} A{fmt(rt)},{fmt(rt * k)} 0 0 0 {fmt(b)},{fmt(top)}" fill="none" '
            f'stroke="{gs}" stroke-width="{fmt(rt * 0.035)}" stroke-linecap="round"/>'
            f'<path d="M{fmt(ai)},{fmt(top)} A{fmt(ri)},{fmt(ri * k)} 0 0 0 {fmt(bi)},{fmt(top)}" fill="none" '
            f'stroke="#ffffff" stroke-opacity="0.8" stroke-width="{fmt(rt * 0.018)}"/>'
        )

    bd, bb = ball(pid + "b", cx, cy, R, glyph_scale=glyph_scale)
    defs = bd + f"""
  <linearGradient id="{pid}-glass" x1="0" y1="0" x2="1" y2="0">
    <stop offset="0" stop-color="#ffffff" stop-opacity="0.55"/>
    <stop offset="0.18" stop-color="#ffffff" stop-opacity="0.12"/>
    <stop offset="0.8" stop-color="#ffffff" stop-opacity="0.08"/>
    <stop offset="1" stop-color="#ffffff" stop-opacity="0.4"/>
  </linearGradient>
  <linearGradient id="{pid}-liq" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="{WHISKY}" stop-opacity="{fmt(LIQ_OPACITY * 0.8)}"/>
    <stop offset="1" stop-color="{WHISKY}" stop-opacity="{fmt(min(1, LIQ_OPACITY * 1.3))}"/>
  </linearGradient>
  <radialGradient id="{pid}-shadow" cx="0.5" cy="0.5" r="0.5">
    <stop offset="0" stop-color="#000" stop-opacity="0.35"/><stop offset="1" stop-color="#000" stop-opacity="0"/>
  </radialGradient>
  <clipPath id="{pid}-liqclip"><path d="{liq_path}"/></clipPath>"""

    out = [
        # 接地影
        f'<ellipse cx="{fmt(cx)}" cy="{fmt(bottom + rb * k * 0.5)}" rx="{fmt(rb * 1.2)}" ry="{fmt(rb * k * 0.75)}" fill="url(#{pid}-shadow)"/>',
        # グラスの胴 (奥の壁)
        f'<path d="{body_path}" fill="#ffffff" fill-opacity="0.18"/>',
        # 液 (球の奥)
        f'<path d="{liq_path}" fill="url(#{pid}-liq)"/>',
        # 液面
        f'<ellipse cx="{fmt(cx)}" cy="{fmt(ly)}" rx="{fmt(lw)}" ry="{fmt(lw * k)}" fill="{SURFACE}" fill-opacity="0.85"/>',
        rim(back=True),
        bb,
    ]

    # 球の沈んでいる部分 (喫水線の手前側より下) に液の色をかぶせ、喫水線を明るく描く
    d = (cy - ly) / CA  # 球の中心から見た液面の高さ
    if -R < d < R:
        rw = math.sqrt(R * R - d * d)
        defs += (
            f'\n  <clipPath id="{pid}-sub"><path clip-rule="evenodd" d="M{fmt(cx - R * 1.1)},{fmt(ly)} H{fmt(cx + R * 1.1)} '
            f'V{fmt(cy + R * 1.1)} H{fmt(cx - R * 1.1)} Z M{fmt(cx - rw)},{fmt(ly)} A{fmt(rw)},{fmt(rw * k)} 0 0 0 {fmt(cx + rw)},{fmt(ly)} Z"/></clipPath>'
        )
        out.append(
            f'<g clip-path="url(#{pid}b-clip)"><rect x="{fmt(cx - R)}" y="{fmt(cy - R)}" width="{fmt(2 * R)}" height="{fmt(2 * R)}" '
            f'fill="{WHISKY}" fill-opacity="{fmt(LIQ_OPACITY * 0.5)}" clip-path="url(#{pid}-sub)"/></g>'
        )
        out.append(
            f'<path d="M{fmt(cx - rw)},{fmt(ly)} A{fmt(rw)},{fmt(rw * k)} 0 0 0 {fmt(cx + rw)},{fmt(ly)}" fill="none" '
            f'stroke="{SURFACE}" stroke-width="{fmt(R * 0.035)}" clip-path="url(#{pid}b-clip)"/>'
        )

    # 泡 (球に重ならない所へ)
    rnd = random.Random(seed)
    bub = []
    for _ in range(bubbles):
        for _try in range(40):
            y = rnd.uniform(ly + (ib - ly) * 0.08, ib - 4)
            w = half_w(y) - wall * 3
            x = cx + rnd.uniform(-w, w)
            r = rnd.uniform(0.012, 0.03) * rt * 2
            if math.hypot(x - cx, y - cy) > R + r * 1.5:
                break
        else:
            continue
        bub.append(
            f'<circle cx="{fmt(x)}" cy="{fmt(y)}" r="{fmt(r)}" fill="#ffffff" fill-opacity="0.35" stroke="#ffffff" '
            f'stroke-opacity="0.85" stroke-width="{fmt(r * 0.3)}"/>'
        )
    out.append(f'<g clip-path="url(#{pid}-liqclip)">' + "".join(bub) + "</g>")

    # 手前の壁・底・縁
    out.append(f'<path d="{body_path}" fill="url(#{pid}-glass)"/>')
    out.append(
        f'<path d="M{fmt(cx - rib)},{fmt(ib)} A{fmt(rib)},{fmt(rib * k)} 0 0 0 {fmt(cx + rib)},{fmt(ib)}" fill="none" '
        f'stroke="#ffffff" stroke-opacity="0.7" stroke-width="{fmt(rt * 0.02)}"/>'
    )
    # 外形の線は側面と底だけ (縁の奥側は球の後ろに描いてある)
    out.append(
        f'<path d="M{fmt(cx - rt)},{fmt(top)} L{fmt(cx - rb)},{fmt(bottom)} A{fmt(rb)},{fmt(rb * k)} 0 0 0 {fmt(cx + rb)},{fmt(bottom)} '
        f'L{fmt(cx + rt)},{fmt(top)}" fill="none" stroke="{gs}" stroke-width="{fmt(rt * 0.035)}" stroke-linejoin="round"/>'
    )
    out.append(rim(back=False))
    # 左の映り込み
    out.append(
        f'<path d="M{fmt(cx - rt * 0.8)},{fmt(top + rt * k * 1.4)} L{fmt(cx - rb * 0.8)},{fmt(bottom - (bottom - top) * 0.14)}" '
        f'stroke="#ffffff" stroke-opacity="0.75" stroke-width="{fmt(rt * 0.06)}" stroke-linecap="round"/>'
    )
    return defs, "\n  ".join(out)


# ---------------------------------------------------------------- アプリアイコン

def app_icons():
    # 縁の奥 (36) から接地影の下端 (約 994) までがキャンバスに収まる大きさ
    d, b = glass_in("a", 512, 195, 815, 390, 352, 306, level=0.34, bubbles=10)
    write("app.svg", svg(1024, d, b))

    # macOS: 1024 のキャンバスに 824 の角丸正方形 (Big Sur 以降の格子) を置き、同じ絵を載せる
    bg = """
  <linearGradient id="m-bg" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#fbf6ea"/><stop offset="1" stop-color="#e6dbc2"/>
  </linearGradient>
  <filter id="m-drop" x="-10%" y="-10%" width="120%" height="130%">
    <feDropShadow dx="0" dy="10" stdDeviation="14" flood-opacity="0.35"/>
  </filter>"""
    sq = '<rect x="100" y="100" width="824" height="824" rx="185" fill="url(#m-bg)" filter="url(#m-drop)"/>'
    d, b = glass_in("m", 512, 225, 765, 330, 300, 250, level=0.34, bubbles=8)
    write("app-macos.svg", svg(1024, bg + d, sq + "\n  " + b))


def app_icon_16():
    """16〜20px 用。16 の格子に合わせて描く (縁・液・球・T だけ)"""
    gs, ink = "#5f6875", CHROME["ink"]
    body = "\n  ".join([
        # 胴 (奥の壁) と液
        '<path d="M1.5,3.5 L2.5,14.5 H13.5 L14.5,3.5" fill="#ffffff" fill-opacity="0.35"/>',
        f'<path d="M2.05,9.5 L2.5,14.5 H13.5 L13.95,9.5 Z" fill="{WHISKY}" fill-opacity="0.75"/>',
        # 球と天面、T (1px 単位に揃える)
        '<circle cx="8" cy="8.5" r="4.6" fill="#d9dde3" stroke="#8e96a2" stroke-width="0.8"/>',
        '<ellipse cx="8" cy="5.3" rx="1.8" ry="0.7" fill="#ffffff"/>',
        f'<rect x="5" y="6.5" width="6" height="1.5" fill="{ink}"/>',
        f'<rect x="7" y="6.5" width="2" height="5" fill="{ink}"/>',
        # 球の沈んだ部分に液の色
        f'<path d="M3.5,10.5 H12.5 L12.6,13.5 H3.4 Z" fill="{WHISKY}" fill-opacity="0.35"/>',
        # 手前の壁と縁
        f'<path d="M1.5,3.5 L2.5,14.5 H13.5 L14.5,3.5" fill="none" stroke="{gs}" stroke-width="1" stroke-linejoin="round"/>',
        f'<ellipse cx="8" cy="3.5" rx="6.5" ry="1.7" fill="none" stroke="{gs}" stroke-width="1"/>',
    ])
    write(
        "app-16.svg",
        f'<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16" viewBox="0 0 16 16">\n  {body}\n</svg>\n',
    )


# ---------------------------------------------------------------- 関連付けアイコン

FILETYPES = [
    # (拡張子, 帯の色, 中身の見本行)
    ("typ", "#1f8a9a", ["= Heading", "#set text(", "Lorem ipsum", "$ x^2 $"]),
    ("md", "#4b4fb8", ["# Heading", "- item", "**bold**", "[link](..)"]),
    ("txt", "#6b7280", ["Lorem ipsum", "dolor sit", "amet, consec", "tetur adip"]),
    ("tbdg", "#c2621b", ["[A] --> [B]", "   |", "<C> --> [D]", "   |"]),
]


def doc_icon(ext, color, lines, pid):
    """256 基準の書類アイコン (1024 で書き出す)。右上が折れた紙 + 見本行 + 球 + 拡張子帯"""
    s = 4
    x0, y0, x1, y1, fold = 44, 12, 212, 244, 50

    def P(x, y):
        return f"{fmt(x * s)},{fmt(y * s)}"

    page = (
        f"M{P(x0 + 10, y0)} L{P(x1 - fold, y0)} L{P(x1, y0 + fold)} L{P(x1, y1 - 10)} Q{P(x1, y1)} {P(x1 - 10, y1)} "
        f"L{P(x0 + 10, y1)} Q{P(x0, y1)} {P(x0, y1 - 10)} L{P(x0, y0 + 10)} Q{P(x0, y0)} {P(x0 + 10, y0)} Z"
    )
    foldp = f"M{P(x1 - fold, y0)} L{P(x1 - fold, y0 + fold - 8)} Q{P(x1 - fold, y0 + fold)} {P(x1 - fold + 8, y0 + fold)} L{P(x1, y0 + fold)} Z"
    defs = f"""
  <linearGradient id="{pid}-page" x1="0" y1="0" x2="0" y2="1">
    <stop offset="0" stop-color="#ffffff"/><stop offset="1" stop-color="#eef0f3"/>
  </linearGradient>
  <filter id="{pid}-drop" x="-10%" y="-10%" width="120%" height="125%">
    <feDropShadow dx="0" dy="6" stdDeviation="8" flood-opacity="0.28"/>
  </filter>"""
    body = [
        f'<path d="{page}" fill="url(#{pid}-page)" stroke="#b8bec8" stroke-width="5" filter="url(#{pid}-drop)"/>',
        f'<path d="{foldp}" fill="#dde1e7" stroke="#b8bec8" stroke-width="5" stroke-linejoin="round"/>',
    ]
    for i, t in enumerate(lines):
        esc = t.replace("&", "&amp;").replace("<", "&lt;")
        body.append(
            f'<text x="{fmt((x0 + 18) * s)}" y="{fmt((y0 + 34 + i * 17) * s)}" font-family="{FONT}" font-weight="bold" '
            f'font-size="{fmt(12 * s)}" fill="{color if i == 0 else "#8a919c"}">{esc}</text>'
        )
    d2, b2 = ball(pid + "b", (x0 + x1) / 2 * s, 148 * s, 38 * s)
    defs += d2
    body.append(b2)
    by = y1 - 52
    body.append(
        f'<path d="M{P(x0, by)} L{P(x1, by)} L{P(x1, y1 - 10)} Q{P(x1, y1)} {P(x1 - 10, y1)} L{P(x0 + 10, y1)} '
        f'Q{P(x0, y1)} {P(x0, y1 - 10)} Z" fill="{color}"/>'
    )
    body.append(
        f'<text x="{fmt((x0 + x1) / 2 * s)}" y="{fmt((by + 37) * s)}" font-family="Segoe UI, Helvetica, Arial, sans-serif" '
        f'font-weight="bold" font-size="{fmt(34 * s)}" text-anchor="middle" fill="#ffffff" letter-spacing="{fmt(2 * s)}">{ext.upper()}</text>'
    )
    return svg(1024, defs, "\n  ".join(body))


def doc_icon_small(ext, color, pid, label=True):
    """16〜48px 向け (32 基準)。紙 + 単色の球 + 色帯。label なら帯に拡張子を入れる (24px 以上向け)"""
    s = 32
    page = f"M{4*s},{1*s} H{20*s} L{28*s},{9*s} V{30*s} a{1*s},{1*s} 0 0 1 {-1*s},{1*s} H{5*s} a{1*s},{1*s} 0 0 1 {-1*s},{-1*s} Z"
    if label:
        band_y, bx, by, br = 18, 12.5, 9.6, 6.2
    else:
        band_y, bx, by, br = 21, 12, 11.5, 6.8
    d2, b2 = ball(pid, bx * s, by * s, br * s, glyph="", solid=color)
    defs = d2 + f'\n  <clipPath id="{pid}-page"><path d="{page}"/></clipPath>'
    body = [
        f'<path d="{page}" fill="#ffffff"/>',
        f'<rect x="0" y="{band_y*s}" width="{32*s}" height="{14*s}" fill="{color}" clip-path="url(#{pid}-page)"/>',
        f'<path d="M{20*s},{1*s} V{9*s} H{28*s}" fill="#dde1e7" stroke="#8a919c" stroke-width="{s}" stroke-linejoin="round"/>',
        f'<path d="{page}" fill="none" stroke="#8a919c" stroke-width="{s}"/>',
        b2,
    ]
    if label:
        body.append(
            f'<text x="{16*s}" y="{(28.4 if len(ext) <= 3 else 27.8)*s}" font-family="Segoe UI, Helvetica, Arial, sans-serif" font-weight="bold" '
            f'font-size="{(9.6 if len(ext) <= 3 else 7.6)*s}" text-anchor="middle" fill="#ffffff">{ext.upper()}</text>'
        )
    return svg(1024, defs, "\n  ".join(body))


def doc_icons():
    for ext, color, lines in FILETYPES:
        write(f"doc-{ext}.svg", doc_icon(ext, color, lines, "d" + ext))
        write(f"doc-{ext}-small.svg", doc_icon_small(ext, color, "s" + ext))
        write(f"doc-{ext}-16.svg", doc_icon_small(ext, color, "t" + ext, label=False))


if __name__ == "__main__":
    for f in os.listdir(OUT):  # 前回の生成物を消してから作り直す
        if f.endswith(".svg"):
            os.remove(os.path.join(OUT, f))
    app_icons()
    app_icon_16()
    doc_icons()
    print("ok:", OUT)
