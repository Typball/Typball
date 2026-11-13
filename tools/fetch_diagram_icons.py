"""図エディタ (ネットワーク図) のアイコンを取得して crates/typball/assets/diagram/icons/ に置く。

    python tools/fetch_diagram_icons.py

取ってくるもの (どちらも改変せずに置く。ファイル名だけ、機器の種類の名前にそろえる):
- Affinity (https://github.com/ecceman/affinity、Unlicense = パブリックドメイン):
  svg/naked → affinity-naked、svg/circle/blue → affinity-circle、svg/square/blue → affinity-square
  (色は 1 色で描かれているので、図エディタが文字列の置換で好きな色に変える。青だけあればよい)
- isoflow の isopack (https://github.com/markmanx/isopacks の collections/isoflow、MIT): → isoflow

アイコンを足すだけなら、このスクリプトは要らない: icons/<セット>/<種類>.svg を置けば、ビルド時に
crates/typball/build.rs が一覧に入れる (表示名は icons/catalog.json。docs/diagram.md の「アイコン」)。
"""

import json
import os
import re
import urllib.request

ROOT = os.path.normpath(os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."))
OUT = os.path.join(ROOT, "crates", "typball", "assets", "diagram", "icons")

# 種類の名前をセットの間でそろえる (同じ機器を、絵柄だけ替えられるように)
ALIASES = {
    "switch-module": "switch",
    "desktop": "client",
    "mobiledevice": "mobile",
    "scull": "skull",
    # Affinity の naked だけ名前が違うもの
    "wifi": "wireless",
    "tj45": "rj45",
    # Affinity の square だけ名前が違うもの
    "servercluster": "server-cluster",
}


def get(url):
    return urllib.request.urlopen(url, timeout=120).read()


def tree(repo, branch):
    data = json.loads(get(f"https://api.github.com/repos/{repo}/git/trees/{branch}?recursive=1"))
    assert not data.get("truncated")
    return [t["path"] for t in data["tree"] if t["type"] == "blob"]


def kind_of(file_name, prefix="", color=""):
    name = file_name[:-4]
    if prefix and name.startswith(prefix):
        name = name[len(prefix):]
    if color and name.endswith("_" + color):
        name = name[: -len(color) - 1]
    name = re.sub(r"[^a-z0-9]+", "-", name.lower()).strip("-")
    return ALIASES.get(name, name)


def fetch(set_id, repo, branch, folder, prefix="", color=""):
    paths = [p for p in tree(repo, branch) if p.startswith(folder + "/") and p.endswith(".svg")]
    out = os.path.join(OUT, set_id)
    os.makedirs(out, exist_ok=True)
    seen = {}
    # 色の付かない名前 (c_shield.svg) と色付きの名前 (c_shield_blue.svg) の両方があるものは、色付きの方を使う
    for path in sorted(paths, key=lambda p: (not p.endswith(f"_{color}.svg"), p)):
        kind = kind_of(os.path.basename(path), prefix, color)
        assert kind, (set_id, path)
        if kind in seen:
            assert color and not path.endswith(f"_{color}.svg"), (set_id, path, kind, seen[kind])
            continue
        seen[kind] = path
        data = get(f"https://raw.githubusercontent.com/{repo}/{branch}/{path}")
        with open(os.path.join(out, kind + ".svg"), "wb") as f:
            f.write(data)
    print(f"{set_id}: {len(seen)} icons")


def main():
    os.makedirs(OUT, exist_ok=True)
    fetch("affinity-naked", "ecceman/affinity", "master", "svg/naked")
    fetch("affinity-circle", "ecceman/affinity", "master", "svg/circle/blue", "c_", "blue")
    fetch("affinity-square", "ecceman/affinity", "master", "svg/square/blue", "sq_", "blue")
    fetch("isoflow", "markmanx/isopacks", "main", "collections/isoflow/icons")
    with open(os.path.join(OUT, "LICENSE-affinity.txt"), "wb") as f:
        f.write(get("https://raw.githubusercontent.com/ecceman/affinity/master/LICENSE"))
    with open(os.path.join(OUT, "LICENSE-isoflow.txt"), "wb") as f:
        f.write(get("https://raw.githubusercontent.com/markmanx/isopacks/main/collections/isoflow/LICENSE"))


if __name__ == "__main__":
    main()
