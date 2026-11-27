#!/usr/bin/env python3
"""Çeviri dosyalarını güncelle: Slint (@tr) ve Rust (tr!/trn!) metinlerini toplar,
lang/<dil>/LC_MESSAGES/vynz.po dosyalarına ekler; var olan çevirileri korur,
artık kullanılmayanları siler. Eksik çevirileri listeler.

Kullanım: python3 lang/update.py   (slint-tr-extractor gerekir: cargo install slint-tr-extractor)
"""
import glob, os, re, subprocess, sys, tempfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.chdir(ROOT)

# Rust'ta sabit tablolardan çalışma anında çevrilen metinler (tr! dışında)
EXTRA = [
    "Bugün için karışım", "Senin için karışım", "Kaldığın yerden", "Yeni keşifler",
    "En çok dinlediklerin", "Beğendiklerin", "Hızlı erişim", "Sana özel",
    "Trend şarkılar", "Ruh hâli ve türler", "Yeni albümler", "Türkiye'de popüler sanatçılar",
    "Keşfet · Türkiye'de trend", "Zevkine göre çalma listeleri",
    "Albümler", "Sanatçılar", "Listeler",
    # src/resolver.py sanatçı sayfası rafları
    "Single ve EP'ler", "Benzer sanatçılar",
]


def unq(s):
    return s.encode().decode("unicode_escape").encode("latin-1").decode("utf-8") if "\\" in s else s


def rust_msgs():
    one, plural = [], []
    for f in sorted(glob.glob("src/*.rs")):
        src = "\n".join(l for l in open(f, encoding="utf-8") if not l.lstrip().startswith("//"))
        for m in re.finditer(r'\btr!\(\s*"((?:[^"\\]|\\.)*)"', src):
            one.append(m.group(1).replace('\\"', '"'))
        for m in re.finditer(r'\btrn!\(\s*"((?:[^"\\]|\\.)*)"', src):
            plural.append(m.group(1).replace('\\"', '"'))
    # GENRES görünen adları
    rec = open("src/recommend.rs", encoding="utf-8").read()
    for m in re.finditer(r'\("[^"]+",\s*"([^"]+)",\s*"[^"]*"\)', rec):
        one.append(m.group(1))
    one += EXTRA
    return one, plural


def parse_po(path):
    """[(msgid, msgid_plural or None, [msgstr...])], sırayla."""
    out = []
    if not os.path.exists(path):
        return out
    cur = None
    field = None
    ctx = None
    for line in open(path, encoding="utf-8"):
        l = line.strip()
        if not l or l.startswith("#"):
            continue
        def val(x):
            return x[x.index('"') + 1: x.rindex('"')].replace('\\"', '"').replace("\\n", "\n").replace("\\\\", "\\")
        if l.startswith("msgctxt "):
            ctx = val(l); continue
        if l.startswith("msgid_plural "):
            cur[1] = val(l); field = ("p",)
        elif l.startswith("msgid "):
            cur = [val(l), None, {}, ctx]; out.append(cur); field = ("id",); ctx = None
        elif l.startswith("msgstr["):
            k = int(l[7:l.index("]")]); cur[2][k] = val(l); field = ("s", k)
        elif l.startswith("msgstr "):
            cur[2][0] = val(l); field = ("s", 0)
        elif l.startswith('"') and cur is not None:
            v = val(l)
            if field[0] == "id": cur[0] += v
            elif field[0] == "p": cur[1] = (cur[1] or "") + v
            else: cur[2][field[1]] = cur[2].get(field[1], "") + v
    return out


def esc(s):
    return s.replace("\\", "\\\\").replace('"', '\\"').replace("\n", "\\n")


def main():
    with tempfile.TemporaryDirectory() as d:
        pot = os.path.join(d, "ui.pot")
        exe = os.path.expanduser("~/.cargo/bin/slint-tr-extractor")
        subprocess.run([exe if os.path.exists(exe) else "slint-tr-extractor", "--no-default-translation-context",
                        "-o", pot] + sorted(glob.glob("ui/*.slint")), check=True)
        slint = [(e[0], e[1], e[3]) for e in parse_po(pot) if e[0]]
    r1, rp = rust_msgs()
    want, seen = [], set()
    for mid, pl, cx in slint + [(m, None, None) for m in r1] + [(m, m, None) for m in rp]:
        if mid and (cx, mid) not in seen:
            seen.add((cx, mid)); want.append((mid, pl, cx))
    for msg_dir in sorted(glob.glob("lang/*/LC_MESSAGES")):
        lang = msg_dir.split(os.sep)[1]
        path = os.path.join(msg_dir, "vynz.po")
        old = {(e[3], e[0]): e for e in parse_po(path)}
        missing = []
        lines = ['msgid ""', 'msgstr ""', '"Content-Type: text/plain; charset=UTF-8\\n"',
                 f'"Language: {lang}\\n"', '"Plural-Forms: nplurals=2; plural=(n != 1);\\n"', ""]
        for mid, pl, cx in want:
            o = old.get((cx, mid))
            tr = o[2] if o else {}
            if not any(tr.values()):
                missing.append(mid)
            if cx:
                lines.append(f'msgctxt "{esc(cx)}"')
            lines.append(f'msgid "{esc(mid)}"')
            if pl:
                lines.append(f'msgid_plural "{esc(pl)}"')
                lines.append(f'msgstr[0] "{esc(tr.get(0, ""))}"')
                lines.append(f'msgstr[1] "{esc(tr.get(1, ""))}"')
            else:
                lines.append(f'msgstr "{esc(tr.get(0, ""))}"')
            lines.append("")
        os.makedirs(os.path.dirname(path), exist_ok=True)
        open(path, "w", encoding="utf-8").write("\n".join(lines))
        print(f"{lang}: {len(want)} metin, {len(missing)} çevrilmemiş")
        if "-v" in sys.argv:
            for m in missing:
                print("  ", m)


if __name__ == "__main__":
    main()
