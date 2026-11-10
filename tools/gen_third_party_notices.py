"""THIRD-PARTY-NOTICES.txt (実行ファイルに入っている第三者のフォント・ソフトウェアのライセンス) を作る。

    python tools/gen_third_party_notices.py

元にするもの:
- Noto Sans JP: crates/typball-core/fonts/LICENSE-NotoSansJP.txt
- UDEV Gothic (と元の BIZ UDゴシック・JetBrains Mono): crates/typball/fonts/LICENSE-UDEVGothic.txt
- Typst 同梱のフォントとデータ: typst-assets クレートの NOTICE
- Typst: Apache License 2.0 (typst-assets に入っている同じ全文を使う)
- wxWidgets と、wxWidgets が内蔵するライブラリ (Scintilla / Lexilla・libpng・zlib・libjpeg・libtiff・Expat・
  NanoSVG・PCRE2・WebView2 SDK): ビルドで展開された wxWidgets のソース (target/release/wxWidgets か WXWIDGETS_DIR)
- Rust クレート: cargo metadata で typball / typball-core から通常の依存 (build / dev 依存は除く) をたどり、
  ~/.cargo/registry に展開された各クレートの LICENSE* / COPYING* / NOTICE* を集める。
  "MIT OR Apache-2.0" のように選べるものは MIT の全文だけを載せる。同じ全文は 1 回だけ載せてクレートを並べる

依存を更新したら (Cargo.lock や wxWidgets の版が変わったら) 作り直してコミットする。
cargo build を一度通して (registry とwxWidgets のソースがそろった状態で) 実行すること。
"""

import glob
import json
import os
import re
import subprocess
import sys

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
OUT = os.path.join(ROOT, "THIRD-PARTY-NOTICES.txt")
RULE = "=" * 80
SUBRULE = "-" * 80
WORKSPACE = {"typball", "typball-core"}


def read(path):
    with open(path, encoding="utf-8", errors="replace") as f:
        return f.read().replace("\r\n", "\n").replace("\t", "    ").strip("\n")


def metadata():
    return json.loads(
        subprocess.run(
            ["cargo", "metadata", "--format-version", "1", "--locked"],
            cwd=ROOT, check=True, capture_output=True, text=True, encoding="utf-8",
        ).stdout
    )


def wx_dir():
    cands = [os.environ.get("WXWIDGETS_DIR", ""), os.path.join(ROOT, "target", "release", "wxWidgets"),
             os.path.join(ROOT, "target", "debug", "wxWidgets")]
    for c in cands:
        if c and os.path.exists(os.path.join(c, "docs", "licence.txt")):
            return c
    sys.exit("wxWidgets のソースが見つかりません (一度ビルドするか WXWIDGETS_DIR を指定してください)")


# ---------------------------------------------------------------- Rust クレート

LICENSE_FILE = re.compile(r"^(LICEN[CS]E|COPYING|COPYRIGHT|NOTICE|UNLICENSE)([-._ ].*)?$", re.I)
# 選べるライセンス ("A OR B") で MIT を選んだときに載せなくてよい全文の種類
ALTERNATIVES = {"APACHE", "UNLICENSE", "ZLIB", "BSL", "ISC", "LGPL", "BSD", "CC0", "0BSD"}


def crate_dir(pkg):
    d = os.path.dirname(pkg["manifest_path"])
    if pkg.get("source") is None:  # vendor/ などのパス依存は、同じ版の registry のコピーも見る
        reg = glob.glob(os.path.join(os.path.expanduser("~"), ".cargo", "registry", "src", "*",
                                     f"{pkg['name']}-{pkg['version']}"))
        if reg and not license_files(d):
            return reg[0]
    return d


def license_files(d):
    found = []
    for name in sorted(os.listdir(d)):
        p = os.path.join(d, name)
        if os.path.isfile(p) and LICENSE_FILE.match(name):
            found.append(p)
        elif os.path.isdir(p) and name.upper() in ("LICENSES", "LICENSE"):
            found += [os.path.join(p, f) for f in sorted(os.listdir(p)) if os.path.isfile(os.path.join(p, f))]
    return found


def file_kind(path, text):
    name = os.path.basename(path).upper()
    if name.startswith("NOTICE"):
        return "NOTICE"
    for key, kind in (("MIT", "MIT"), ("APACHE", "APACHE"), ("UNLICENSE", "UNLICENSE"), ("ZLIB", "ZLIB"),
                      ("BSL", "BSL"), ("BOOST", "BSL"), ("ISC", "ISC"), ("LGPL", "LGPL"), ("0BSD", "0BSD"),
                      ("BSD", "BSD"), ("CC0", "CC0"), ("UNICODE", "UNICODE")):
        if key in name:
            return kind
    # LICENSE / COPYING のような名前は中身で見分ける
    if "Apache License" in text and "Version 2.0" in text:
        return "APACHE"
    if "Permission is hereby granted, free of charge" in text:
        return "MIT"
    return "OTHER"


def pick_texts(pkg, files):
    """クレートの全文の中から載せるものを選ぶ"""
    expr = (pkg.get("license") or "").replace("/", " OR ")
    tokens = set(re.findall(r"[A-Za-z0-9.+-]+", expr))
    texts = []
    for f in files:
        t = read(f)
        if t.strip():
            texts.append((file_kind(f, t), t))
    if "OR" in tokens and "MIT" in tokens and any(k == "MIT" for k, _ in texts):
        kept = [(k, t) for k, t in texts if k not in ALTERNATIVES]
        if kept:
            return kept
    return texts


def rust_crates(meta):
    pk = {p["id"]: p for p in meta["packages"]}
    nodes = {n["id"]: n for n in meta["resolve"]["nodes"]}
    roots = [p["id"] for p in meta["packages"] if p["name"] in WORKSPACE]
    seen, stack = set(roots), list(roots)
    while stack:
        for dep in nodes[stack.pop()]["deps"]:
            if any(k["kind"] is None for k in dep["dep_kinds"]) and dep["pkg"] not in seen:
                seen.add(dep["pkg"])
                stack.append(dep["pkg"])
    crates = sorted((pk[i] for i in seen if pk[i]["name"] not in WORKSPACE), key=lambda p: (p["name"], p["version"]))

    groups = {}  # 正規化した全文 → (全文, [クレート])
    missing = []
    for p in crates:
        texts = pick_texts(p, license_files(crate_dir(p)))
        if not texts:
            missing.append(p)
            continue
        body = ("\n\n" + SUBRULE + "\n\n").join(t for _, t in texts)
        key = re.sub(r"\s+", " ", body).strip()
        groups.setdefault(key, (body, []))[1].append(p)
    return crates, groups, missing


def crate_line(p):
    return f"  {p['name']} {p['version']} ({p.get('license') or 'see text'})  https://crates.io/crates/{p['name']}/{p['version']}"


# ---------------------------------------------------------------- 組み立て

def main():
    meta = metadata()
    pk = {p["name"]: p for p in meta["packages"]}
    assets = pk["typst-assets"]
    assets_dir = os.path.dirname(assets["manifest_path"])
    typst_ver = pk["typst"]["version"]
    wx = wx_dir()
    wx_ver = read(os.path.join(wx, "include", "wx", "version.h"))
    wx_version = ".".join(
        wx_ver.split(f"#define {k}")[1].split()[0] for k in ("wxMAJOR_VERSION", "wxMINOR_VERSION", "wxRELEASE_NUMBER")
    )
    crates, groups, missing = rust_crates(meta)

    # wxWidgets が内蔵して静的リンクしているライブラリ (wxUSE_* = builtin) と WebView2 SDK
    jpeg_readme = read(os.path.join(wx, "src", "jpeg", "README"))
    jpeg_legal = jpeg_readme[jpeg_readme.index("LEGAL ISSUES\n============"):jpeg_readme.index("REFERENCES\n==========")].strip()
    wx_libs = [
        ("Scintilla and Lexilla (https://www.scintilla.org/)", ["src/stc/scintilla/License.txt"]),
        ("libpng (http://www.libpng.org/)", ["src/png/LICENSE"]),
        ("zlib (https://zlib.net/)", ["src/zlib/LICENSE"]),
        ("libjpeg (Independent JPEG Group, https://ijg.org/)\n"
         "   This software is based in part on the work of the Independent JPEG Group.", None),
        ("libtiff (https://libtiff.gitlab.io/libtiff/)", ["src/tiff/LICENSE.md"]),
        ("Expat (https://libexpat.github.io/)", ["src/expat/expat/COPYING"]),
        ("NanoSVG (https://github.com/memononen/nanosvg)", ["3rdparty/nanosvg/LICENSE.txt"]),
        ("PCRE2 (https://github.com/PCRE2Project/pcre2)", ["3rdparty/pcre/LICENCE.md"]),
        ("Microsoft Edge WebView2 SDK (https://aka.ms/webview2)",
         ["3rdparty/webview2/LICENSE.txt", "3rdparty/webview2/NOTICE.txt"]),
    ]

    parts = [
        "Typball - Third-party notices / サードパーティのライセンス",
        "",
        "Typball itself is licensed under the MIT License (see LICENSE).",
        "The Typball executable also contains the following third-party fonts and software.",
        "Typball 本体は MIT License です (LICENSE を参照)。実行ファイルには次の第三者のフォントと",
        "ソフトウェアが含まれており、それぞれ下記のライセンスに従います。",
        "",
        "  1. Bundled Japanese fonts",
        "     - Noto Sans JP (Regular / Bold) ........ SIL Open Font License 1.1",
        "     - UDEV Gothic (Regular / Bold) ......... SIL Open Font License 1.1",
        "       (composite of BIZ UDGothic and JetBrains Mono, both SIL Open Font License 1.1)",
        f"  2. Fonts and data bundled with Typst (typst-assets {assets['version']})",
        "     - Libertinus Serif ..................... SIL Open Font License 1.1",
        "     - New Computer Modern / NewCM Math ..... GUST Font License 1.0",
        "       (NewCM10-Regular: GPL-3.0-or-later with Font Exception and Distribution Exception)",
        "     - DejaVu Sans Mono ..................... Bitstream Vera / Arev Fonts license (DejaVu changes: public domain)",
        "     - Foxit PDF standard fonts ............. BSD-3-Clause (PDFium Authors)",
        "     - ICC profiles, HTML / MathML data ..... CC0 1.0 / CC BY 4.0 / BSD-3-Clause / W3C",
        f"  3. Typst {typst_ver} ............................ Apache License 2.0",
        f"  4. wxWidgets {wx_version} ......................... wxWindows Library Licence 3.1",
        "  5. Libraries built into wxWidgets",
        "     - Scintilla / Lexilla .................. Scintilla License (HPND-style)",
        "     - libpng ............................... PNG Reference Library License v2",
        "     - zlib ................................. zlib License",
        "     - libjpeg (IJG) ........................ IJG License",
        "     - libtiff .............................. libtiff License",
        "     - Expat ................................ MIT",
        "     - NanoSVG .............................. zlib License",
        "     - PCRE2 ................................ BSD-3-Clause with PCRE2 exception",
        "     - Microsoft Edge WebView2 SDK .......... BSD-3-Clause",
        f"  6. Rust crates ({len(crates)} crates) ............... MIT, Apache-2.0, Unicode-3.0, BSD, ISC, Zlib, MPL-2.0 and others",
        "     For crates offered under a choice of licenses (e.g. \"MIT OR Apache-2.0\"), Typball uses them under MIT,",
        "     or under Apache-2.0 when the crate does not ship the MIT text.",
        "     選べるライセンス (\"MIT OR Apache-2.0\" など) のクレートは MIT で (MIT の全文が無いものは Apache-2.0 で) 利用しています。",
        "     Source code of every crate is available at the crates.io URL listed with it.",
        "",
    ]

    def section(title, body):
        parts.extend([RULE, title, RULE, "", body, ""])

    section(
        "1.1 Noto Sans JP (https://github.com/notofonts/noto-cjk)",
        read(os.path.join(ROOT, "crates", "typball-core", "fonts", "LICENSE-NotoSansJP.txt")),
    )
    section(
        "1.2 UDEV Gothic (https://github.com/yuru7/udev-gothic)",
        read(os.path.join(ROOT, "crates", "typball", "fonts", "LICENSE-UDEVGothic.txt")),
    )
    section(
        f"2. Fonts and data bundled with Typst (typst-assets {assets['version']}, https://github.com/typst/typst-assets)\n"
        "   The following is the NOTICE file of typst-assets, reproduced verbatim.",
        read(os.path.join(assets_dir, "NOTICE")),
    )
    section(
        f"3. Typst {typst_ver} (https://github.com/typst/typst)\n"
        "   Copyright The Typst Project Developers. Licensed under the Apache License, Version 2.0.\n"
        "   (This is also the full Apache License 2.0 text for the Rust crates in section 6.)",
        read(os.path.join(assets_dir, "LICENSE")),
    )
    section(f"4. wxWidgets {wx_version} (https://www.wxwidgets.org/)", read(os.path.join(wx, "docs", "licence.txt")))
    for i, (title, files) in enumerate(wx_libs, 1):
        body = jpeg_legal if files is None else ("\n\n" + SUBRULE + "\n\n").join(
            read(os.path.join(wx, *f.split("/"))) for f in files)
        section(f"5.{i} {title}", body)

    parts.extend([RULE, f"6. Rust crates ({len(crates)} crates)", RULE, ""])
    for i, (body, members) in enumerate(sorted(groups.values(), key=lambda g: g[1][0]["name"]), 1):
        parts.append(f"6.{i} " + ("The following crate is" if len(members) == 1 else "The following crates are")
                     + " distributed under the license text below:")
        parts.extend(crate_line(p) for p in members)
        parts.extend(["", body, "", SUBRULE, ""])
    if missing:
        # 全文を同梱していないクレート。Apache-2.0 を選べるものはそれで使う (全文は第 3 節。著作権者の行が要らない)
        apache = [p for p in missing if "Apache-2.0" in (p.get("license") or "")]
        others = [p for p in missing if p not in apache]
        parts.append(f"6.{len(groups) + 1} The following crates do not ship a license file in their published package.")
        parts.append("   Typball uses them under the Apache License 2.0, whose full text is in section 3:")
        parts.extend(crate_line(p) for p in apache)
        if others:
            parts.extend(["", "   The following are used under the license named in their metadata, which requires no notice:"])
            parts.extend(crate_line(p) for p in others)
        parts.append("")

    with open(OUT, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(parts).rstrip("\n") + "\n")
    print(f"wrote {OUT}: {len(crates)} crates, {len(groups)} distinct license texts, {len(missing)} without files")
    for p in missing:
        print("  no license file:", p["name"], p["version"], p.get("license"))


if __name__ == "__main__":
    main()
