"""アイコンの SVG (gen_icons.py が作る) を PNG に描き、ICO・ICNS・RGBA にまとめる。

SVG の描画は Edge (Chromium) のヘッドレスに任せる (透過背景のスクリーンショットを撮って Pillow で切り出す)。
小さいサイズは縮小ではなくそのサイズで描かせるので、線が潰れにくい。

    python design/icons/gen_icons.py      # SVG を作り直したとき
    python design/icons/export_icons.py

出力 (crates/typball/assets/icons/):
    Typball.ico            Windows のアプリアイコン (exe に埋め込む。build.rs)
    doc-{typ,md,txt,tbdg}.ico   Windows の関連付けアイコン (同上。インストーラーが Typball.exe,-101 などで参照)
    Typball.icns           macOS の .app のアイコン (CI が Contents/Resources に置く)
    doc-{typ,md,txt,tbdg}.icns  macOS の書類アイコン (書類の種類を登録するときに使う)
    Typball-256.rgba       256x256 の生 RGBA (include_bytes! で読む)。バージョン情報の大きなアイコンと
                           Linux のウィンドウアイコン (appicon.rs が必要な大きさに縮める)

必要なもの: Pillow、Edge か Chrome (TYPBALL_BROWSER で場所を指定できる)
"""

import io
import os
import struct
import subprocess
import tempfile

from PIL import Image

HERE = os.path.dirname(os.path.abspath(__file__))
DEST = os.path.normpath(os.path.join(HERE, "..", "..", "crates", "typball", "assets", "icons"))

ICO_SIZES = [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
ICNS_SIZES = [16, 32, 64, 128, 256, 512, 1024]
EXTS = ["typ", "md", "txt", "tbdg"]


# 大きさごとにどの SVG を使うか
def app_win(z):
    return "app-16.svg" if z <= 20 else "app.svg"


def app_mac(z):
    return "app-16.svg" if z <= 16 else "app-macos.svg"


def doc(ext):
    return lambda z: f"doc-{ext}-16.svg" if z <= 16 else (f"doc-{ext}-small.svg" if z <= 48 else f"doc-{ext}.svg")


def browser():
    cands = [
        os.environ.get("TYPBALL_BROWSER", ""),
        r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Microsoft\Edge\Application\msedge.exe",
        r"C:\Program Files\Google\Chrome\Application\chrome.exe",
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/usr/bin/google-chrome",
        "/usr/bin/chromium",
    ]
    for c in cands:
        if c and os.path.exists(c):
            return c
    raise SystemExit("Edge / Chrome が見つかりません (TYPBALL_BROWSER で指定してください)")


def render(jobs):
    """jobs: {(svg, size)} → {(svg, size): RGBA Image}。全部を 1 枚のページに並べて 1 回で撮る"""
    jobs = sorted(set(jobs), key=lambda j: (-j[1], j[0]))
    pad = 8
    place, x, y, row_h, width = {}, pad, pad, 0, 0
    last = None
    for svg, z in jobs:
        if last is not None and z != last:  # 大きさが変わったら改行
            x, y, row_h = pad, y + row_h + pad, 0
        place[(svg, z)] = (x, y)
        x += z + pad
        row_h = max(row_h, z)
        width = max(width, x)
        last = z
    height = y + row_h + pad
    with tempfile.TemporaryDirectory() as tmp:
        imgs = "".join(
            f'<img src="file:///{os.path.join(HERE, svg).replace(os.sep, "/")}" width="{z}" height="{z}" '
            f'style="position:absolute;left:{px}px;top:{py}px">'
            for (svg, z), (px, py) in place.items()
        )
        page = os.path.join(tmp, "sheet.html")
        with open(page, "w", encoding="utf-8") as f:
            f.write(f"<!doctype html><html style='background:transparent'><body style='margin:0;background:transparent'>{imgs}</body></html>")
        shot = os.path.join(tmp, "sheet.png")
        subprocess.run(
            [browser(), "--headless=new", "--disable-gpu", "--hide-scrollbars", "--allow-file-access-from-files",
             "--force-device-scale-factor=1", "--default-background-color=00000000",
             f"--window-size={width},{height}", f"--screenshot={shot}", "file:///" + page.replace(os.sep, "/")],
            check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
        )
        sheet = Image.open(shot).convert("RGBA")
        if sheet.getpixel((0, 0))[3] != 0:
            raise SystemExit("背景が透明になっていません (ブラウザが --default-background-color に対応していない)")
        return {j: sheet.crop((px, py, px + j[1], py + j[1])) for j, (px, py) in place.items()}


def png_bytes(im):
    b = io.BytesIO()
    im.save(b, "PNG", optimize=True)
    return b.getvalue()


def dib_bytes(im):
    """ICO の中の BMP (32bit BGRA・下から上。高さは AND マスク込みで 2 倍と書く)"""
    w, h = im.size
    head = struct.pack("<IiiHHIIiiII", 40, w, h * 2, 1, 32, 0, 0, 0, 0, 0, 0)
    px = im.tobytes("raw", "BGRA")
    rows = [px[r * w * 4:(r + 1) * w * 4] for r in range(h)][::-1]
    mask = b"\0" * (((w + 31) // 32) * 4 * h)
    return head + b"".join(rows) + mask


def write_ico(path, images):
    """256 は PNG、それ未満は BMP で入れる (古い読み手でも確実に読める形)"""
    datas = [png_bytes(im) if im.width >= 256 else dib_bytes(im) for im in images]
    out = struct.pack("<HHH", 0, 1, len(images))
    offset = 6 + 16 * len(images)
    for im, d in zip(images, datas):
        w = im.width if im.width < 256 else 0
        out += struct.pack("<BBBBHHII", w, w, 0, 0, 1, 32, len(d), offset)
        offset += len(d)
    with open(path, "wb") as f:
        f.write(out + b"".join(datas))


ICNS_TYPES = {
    # 大きさ → 入れる種類 (Retina 用の @2x も同じ画像で埋める)
    16: ["icp4"],
    32: ["icp5", "ic11"],
    64: ["icp6", "ic12"],
    128: ["ic07"],
    256: ["ic08", "ic13"],
    512: ["ic09", "ic14"],
    1024: ["ic10"],
}


def write_icns(path, by_size):
    body = b""
    for z, types in ICNS_TYPES.items():
        d = png_bytes(by_size[z])
        for t in types:
            body += t.encode() + struct.pack(">I", 8 + len(d)) + d
    with open(path, "wb") as f:
        f.write(b"icns" + struct.pack(">I", 8 + len(body)) + body)


def main():
    os.makedirs(DEST, exist_ok=True)
    sets = {"Typball.ico": (app_win, ICO_SIZES), "Typball.icns": (app_mac, ICNS_SIZES)}
    for e in EXTS:
        sets[f"doc-{e}.ico"] = (doc(e), ICO_SIZES)
        sets[f"doc-{e}.icns"] = (doc(e), ICNS_SIZES)
    jobs = {(pick(z), z) for pick, sizes in sets.values() for z in sizes}
    shots = render(jobs)

    for name, (pick, sizes) in sets.items():
        path = os.path.join(DEST, name)
        if name.endswith(".ico"):
            write_ico(path, [shots[(pick(z), z)] for z in sizes])
        else:
            write_icns(path, {z: shots[(pick(z), z)] for z in sizes})
        print(f"{name}: {os.path.getsize(path):,} bytes")
    with open(os.path.join(DEST, "Typball-256.rgba"), "wb") as f:
        f.write(shots[(app_win(256), 256)].tobytes("raw", "RGBA"))
    print("Typball-256.rgba")

    # 確認用: 書き出した ICO の各サイズを並べた画像 (target/icon-export-check.png。コミットしない)
    names = ["Typball.ico"] + [f"doc-{e}.ico" for e in EXTS]
    sheet = Image.new("RGBA", (sum(ICO_SIZES) + 12 * len(ICO_SIZES) + 12, (256 + 12) * len(names) + 12), (255, 255, 255, 255))
    for r, name in enumerate(names):
        x = 12
        for z in ICO_SIZES:
            im = shots[(sets[name][0](z), z)]
            sheet.alpha_composite(im, (x, 12 + r * 268 + 256 - z))
            x += z + 12
    check_dir = os.path.join(HERE, "..", "..", "target")
    os.makedirs(check_dir, exist_ok=True)
    check = os.path.normpath(os.path.join(check_dir, "icon-export-check.png"))
    sheet.save(check)
    print("check image:", check)


if __name__ == "__main__":
    main()
