# vynz çözücü yardımcısı: sayfa adresinden doğrudan ses akışı adresi.
# Rust tarafı başlatır, stdin/stdout üzerinden JSON satırlarıyla konuşur ve
# boşta kalınca kapatır. Kullanıcının ~/.config/yt-dlp/config dosyası okunmaz
# (YoutubeDL API'si ayar dosyası okumaz).
#
# İstek : {"rid": 1, "url": "...", "prio": 0, "cookies": "firefox"|null}
#         {"rid": 2, "op": "radio", "id": "<videoId>", "prio": 1}   benzer şarkılar (ytmusicapi)
# Yanıt : {"rid": 1, "ok": true, "stream": "...", "expire": 1700000000, "abr": 272.4, "ext": "webm"}
#         {"rid": 1, "ok": false, "error": "..."}

import json
import os
import re
import queue
import sys
import threading
import time
import urllib.parse

import yt_dlp

FAST_ARGS = {"youtube": {"player_client": ["web_music"], "player_skip": ["webpage", "configs"]}}
DENO_PATH = os.environ.get("VYNZ_DENO") or None

# Rust tarafı çerez tarayıcısını ilk argüman olarak verir ("-" = yok).
WARM_COOKIES = sys.argv[1] if len(sys.argv) > 1 and sys.argv[1] != "-" else None

out_lock = threading.Lock()
jobs = queue.PriorityQueue()   # ön-çözme işleri (öncelik sıralı)
urgent = queue.Queue()         # "şimdi çal": ayrılmış iş parçacığı, ön-çözme yüzünden beklemez
local = threading.local()
seq = 0


def send(obj):
    with out_lock:
        sys.stdout.write(json.dumps(obj) + "\n")
        sys.stdout.flush()


def ydl(fast, cookies):
    # Her iş parçacığı kendi örneğini tutar: çerezler ve oynatıcı önbelleği bir kez yüklenir.
    key = (fast, cookies)
    cache = getattr(local, "cache", None)
    if cache is None:
        cache = local.cache = {}
    if key not in cache:
        opts = {
            "quiet": True,
            "no_warnings": True,
            "noprogress": True,
            "format": "bestaudio/best",
            "skip_download": True,
            "noplaylist": True,
        }
        if cookies:
            opts["cookiesfrombrowser"] = (cookies,)
        if fast:
            opts["extractor_args"] = FAST_ARGS
        if DENO_PATH:
            opts["js_runtimes"] = {"deno": {"path": DENO_PATH}}
        cache[key] = yt_dlp.YoutubeDL(opts)
    return cache[key]


def expire_of(url):
    q = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
    try:
        return int(q["expire"][0])
    except (KeyError, ValueError, IndexError):
        return int(time.time()) + 3 * 3600


def resolve(url, cookies):
    is_yt = "youtube.com" in url or "youtu.be" in url
    attempts = [(True, cookies), (False, cookies)] if is_yt else [(False, None)]
    if is_yt and cookies:
        attempts.append((False, None))  # çerez sorunluysa girişsiz dene
    last = None
    for fast, ck in attempts:
        try:
            info = ydl(fast, ck).extract_info(url, download=False)
            stream = info.get("url")
            if not stream and info.get("requested_formats"):
                stream = info["requested_formats"][0].get("url")
            if not stream:
                raise RuntimeError("akış adresi yok")
            return {
                "stream": stream,
                "expire": expire_of(stream),
                "abr": info.get("abr"),
                "ext": info.get("audio_ext") or info.get("ext"),
                "protocol": info.get("protocol"),
            }
        except Exception as e:  # noqa: BLE001 - bir sonraki yolu dene
            last = e
    raise RuntimeError(str(last).splitlines()[0][:300] if last else "çözülemedi")


def ytmusic():
    y = getattr(local, "ytm", None)
    if y is None:
        from ytmusicapi import YTMusic
        y = local.ytm = YTMusic(language="tr", location="TR")
    return y


def seconds(text):
    try:
        parts = [int(p) for p in str(text).split(":")]
    except ValueError:
        return 0
    s = 0
    for p in parts:
        s = s * 60 + p
    return s


def radio(video_id):
    w = ytmusic().get_watch_playlist(videoId=video_id, radio=True, limit=50)
    out = []
    for t in w.get("tracks") or []:
        vid = t.get("videoId")
        if not vid:
            continue
        thumbs = t.get("thumbnail") or t.get("thumbnails") or []
        thumb = thumbs[-1]["url"] if thumbs else ""
        out.append({
            "id": vid,
            "title": t.get("title") or "",
            "artist": ", ".join(a.get("name", "") for a in (t.get("artists") or [])),
            "album": (t.get("album") or {}).get("name") or "",
            "duration": seconds(t.get("length") or 0),
            "thumb": thumb,
        })
    return out


def thumb_of(x):
    th = x.get("thumbnails") or x.get("thumbnail") or []
    if isinstance(th, dict):
        th = th.get("thumbnails") or []
    return th[-1]["url"] if th else ""


def names(x):
    return ", ".join(a.get("name", "") for a in (x.get("artists") or []) if a.get("name"))


def track_of(t, album_thumb=""):
    vid = t.get("videoId")
    if not vid:
        return None
    return {
        "id": vid,
        "title": t.get("title") or "",
        "artist": names(t),
        "album": (t.get("album") or {}).get("name") if isinstance(t.get("album"), dict) else (t.get("album") or ""),
        "duration": t.get("duration_seconds") or seconds(t.get("duration") or t.get("length") or 0),
        "thumb": thumb_of(t) or album_thumb,
    }


def items(v):
    if isinstance(v, dict):
        return v.get("items") or v.get("results") or []
    return v or []


def explore():
    y = ytmusic()
    e = y.get_explore()
    out = {"trending": [], "releases": [], "moods": [], "artists": []}
    for t in items(e.get("trending")):
        tr = track_of(t)
        if tr:
            out["trending"].append(tr)
    for a in items(e.get("new_releases")):
        if a.get("browseId"):
            out["releases"].append({"kind": "album", "key": a["browseId"], "title": a.get("title") or "", "sub": names(a), "thumb": thumb_of(a)})
    for m in items(e.get("moods_and_genres")):
        if m.get("params"):
            out["moods"].append({"kind": "mood", "key": m["params"], "title": m.get("title") or "", "sub": "", "thumb": ""})
    try:
        c = y.get_charts(country="TR")
        for a in items(c.get("artists"))[:30]:
            if a.get("browseId"):
                out["artists"].append({"kind": "artist", "key": a["browseId"], "title": a.get("title") or "", "sub": a.get("subscribers") or "", "thumb": thumb_of(a)})
    except Exception:  # noqa: BLE001 - listeler isteğe bağlı
        pass
    return out


def collection(kind, key):
    y = ytmusic()
    if kind == "album":
        a = y.get_album(key)
        th = thumb_of(a)
        tracks = [x for x in (track_of(t, th) for t in a.get("tracks") or []) if x]
        for t in tracks:
            t["album"] = t["album"] or a.get("title") or ""
        return {"title": a.get("title") or "", "sub": names(a), "tracks": tracks}
    if kind == "artist":
        return artist_page(y, key)
    if kind == "mood":
        pls = y.get_mood_playlists(key)
        cards = [{"kind": "playlist", "key": p["playlistId"], "title": p.get("title") or "", "sub": p.get("description") or "", "thumb": thumb_of(p)} for p in pls if p.get("playlistId")]
        return {"title": "", "sub": "", "cards": cards}
    p = y.get_playlist(key, limit=200)
    return {"title": p.get("title") or "", "sub": (p.get("author") or {}).get("name") if isinstance(p.get("author"), dict) else "", "tracks": [x for x in (track_of(t) for t in p.get("tracks") or []) if x]}


def artist_page(y, key):
    a = y.get_artist(key)
    songs = a.get("songs") or {}
    res = []
    if songs.get("browseId"):
        try:
            res = y.get_playlist(songs["browseId"], limit=50).get("tracks") or []
        except Exception:  # noqa: BLE001
            res = []
    if not res:
        res = songs.get("results") or []
    name = a.get("name") or ""
    tracks = [x for x in (track_of(t) for t in res) if x][:50]
    for t in tracks:
        t["artist"] = t["artist"] or name

    def releases(v, default):
        out = []
        for r in items(v):
            if not r.get("browseId"):
                continue
            sub = " · ".join(x for x in [r.get("type") or default, str(r.get("year") or "")] if x)
            out.append({"kind": "album", "key": r["browseId"], "title": r.get("title") or "", "sub": sub, "thumb": thumb_of(r)})
        return out

    related = []
    for r in items(a.get("related")):
        if r.get("browseId"):
            related.append({"kind": "artist", "key": r["browseId"], "title": r.get("title") or "", "sub": r.get("subscribers") or "", "thumb": thumb_of(r)})
    shelves = []
    for title, style, cards in (
        ("Albümler", 2, releases(a.get("albums"), "Albüm")),
        ("Single ve EP'ler", 2, releases(a.get("singles"), "Single")),
        ("Benzer sanatçılar", 3, related),
    ):
        if cards:
            shelves.append({"title": title, "style": style, "cards": cards})
    listeners = (a.get("monthlyListeners") or "").replace("Aylık kitle:", "").strip()
    sub = f"Aylık {listeners} dinleyici" if listeners else (f"{a['subscribers']} abone" if a.get("subscribers") else "")
    desc = " ".join((a.get("description") or "").split())
    if len(desc) > 420:
        desc = desc[:420].rsplit(" ", 1)[0] + "…"
    return {
        "title": name,
        "sub": sub,
        "desc": desc,
        "banner": re.sub(r"=w\d+-h\d+", "=w1440-h600", thumb_of(a)),
        "tracks": tracks,
        "shelves": shelves,
    }


def search(q, limit=20):
    y = ytmusic()
    res = y.search(q, limit=limit)
    out = {"top": None, "songs": [], "albums": [], "artists": [], "playlists": []}
    for r in res:
        rt = r.get("resultType")
        top = r.get("category") in ("Top result", "En iyi sonuç") and out["top"] is None
        if rt in ("song", "video"):
            t = track_of(r)
            if not t:
                continue
            if top:
                out["top"] = {"kind": "song", "track": t, "title": t["title"], "sub": "Şarkı · " + t["artist"]}
            # videolar (klipler) de çalınabilir ama şarkılardan sonra gelsin
            out["songs"].append(t) if rt == "song" else out.setdefault("videos", []).append(t)
        elif rt == "album" and r.get("browseId"):
            c = {"kind": "album", "key": r["browseId"], "title": r.get("title") or "", "sub": " · ".join(x for x in [r.get("type") or "Albüm", names(r), str(r.get("year") or "")] if x), "thumb": thumb_of(r)}
            out["albums"].append(c)
            if top:
                out["top"] = {**c, "card": c}
        elif rt == "artist":
            a = (r.get("artists") or [{}])[0]
            key = r.get("browseId") or a.get("id")
            if not key:
                continue
            c = {"kind": "artist", "key": key, "title": r.get("artist") or a.get("name") or "", "sub": "Sanatçı", "thumb": thumb_of(r)}
            out["artists"].append(c)
            if top:
                out["top"] = {**c, "card": c}
        elif rt == "playlist" and r.get("browseId"):
            key = r["browseId"][2:] if r["browseId"].startswith("VL") else r["browseId"]
            c = {"kind": "playlist", "key": key, "title": r.get("title") or "", "sub": "Liste · " + (r.get("author") or ""), "thumb": thumb_of(r)}
            out["playlists"].append(c)
            if top:
                out["top"] = {**c, "card": c}
    out["songs"] += out.pop("videos", [])
    # En iyi sonuç sanatçıysa altındaki ilk şarkılarda sanatçı alanı boş gelir: onunla doldur.
    if out["top"] and out["top"].get("kind") == "artist":
        for t in out["songs"]:
            if not t["artist"]:
                t["artist"] = out["top"]["title"]
    return out


def home():
    y = ytmusic()
    secs = []
    for sec in y.get_home(limit=8):
        sec_items = []
        for it in sec.get("contents") or []:
            if not it:
                continue
            th = thumb_of(it)
            if it.get("videoId"):
                t = track_of(it)
                if t:
                    t["kind"] = "track"
                    sec_items.append(t)
            elif it.get("playlistId"):
                sec_items.append({
                    "kind": "playlist",
                    "key": it["playlistId"],
                    "title": it.get("title") or "",
                    "sub": it.get("description") or "",
                    "thumb": th,
                })
            elif (it.get("browseId") or "").startswith("MPRE"):
                who = names(it)
                sec_items.append({
                    "kind": "album",
                    "key": it["browseId"],
                    "title": it.get("title") or "",
                    "sub": " · ".join(x for x in (who, str(it.get("year") or "")) if x),
                    "thumb": th,
                })
        if sec_items:
            secs.append({"title": sec.get("title") or "", "items": sec_items})
    return secs


def lyrics(video_id):
    y = ytmusic()
    wp = y.get_watch_playlist(videoId=video_id, limit=1)
    bid = (wp or {}).get("lyrics")
    if not bid:
        return {"source": "youtube", "synced": False, "lines": []}
    data = y.get_lyrics(bid, timestamps=True)
    if not data or not data.get("lyrics"):
        return {"source": "youtube", "synced": False, "lines": []}
    raw = data["lyrics"]
    if isinstance(raw, list):
        lines = []
        for ln in raw:
            if isinstance(ln, dict):
                st = ln.get("start_time") or 0
                en = ln.get("end_time") or 0
                tx = ln.get("text") or ""
            else:
                st = getattr(ln, "start_time", None) or 0
                en = getattr(ln, "end_time", None) or 0
                tx = getattr(ln, "text", None) or ""
            lines.append({"t": st / 1000.0, "end": en / 1000.0, "text": tx})
        return {"source": "youtube", "synced": True, "lines": lines}
    lines = [{"t": None, "end": None, "text": t} for t in str(raw).split("\n")]
    return {"source": "youtube", "synced": False, "lines": lines}


def flat_extract(url):
    opts = {
        "quiet": True,
        "no_warnings": True,
        "noprogress": True,
        "extract_flat": "in_playlist",
        "skip_download": True,
    }
    if DENO_PATH:
        opts["js_runtimes"] = {"deno": {"path": DENO_PATH}}
    with yt_dlp.YoutubeDL(opts) as y:
        info = y.extract_info(url, download=False)
    if not isinstance(info, dict):
        raise RuntimeError("liste okunamadı")
    entries = info.get("entries")
    if entries is not None and not isinstance(entries, list):
        entries = list(entries)
    return {"title": info.get("title"), "entries": entries or []}


def handle(req):
    op = req.get("op")
    if op == "flat":
        try:
            send({"rid": req["rid"], "ok": True, "data": flat_extract(req.get("url") or "")})
        except Exception as e:  # noqa: BLE001
            send({"rid": req["rid"], "ok": False, "error": str(e).splitlines()[0][:300] if str(e) else "hata"})
        return
    if op == "search":
        try:
            limit = int(req.get("limit") or 20)
            send({"rid": req["rid"], "ok": True, "data": search(req.get("q") or "", limit=limit)})
        except Exception as e:  # noqa: BLE001
            send({"rid": req["rid"], "ok": False, "error": str(e).splitlines()[0][:300] if str(e) else "hata"})
        return
    if op in ("home", "lyrics"):
        try:
            data = home() if op == "home" else lyrics(req.get("id") or "")
            send({"rid": req["rid"], "ok": True, "data": data})
        except Exception as e:  # noqa: BLE001
            send({"rid": req["rid"], "ok": False, "error": str(e).splitlines()[0][:300] if str(e) else "hata"})
        return
    if op in ("explore", "collection"):
        try:
            data = explore() if op == "explore" else collection(req.get("kind"), req.get("key"))
            send({"rid": req["rid"], "ok": True, "data": data})
        except Exception as e:  # noqa: BLE001
            send({"rid": req["rid"], "ok": False, "error": str(e).splitlines()[0][:300] if str(e) else "hata"})
        return
    if req.get("op") == "radio":
        try:
            send({"rid": req["rid"], "ok": True, "tracks": radio(req["id"])})
        except Exception as e:  # noqa: BLE001
            send({"rid": req["rid"], "ok": False, "error": str(e)[:300]})
        return
    try:
        res = resolve(req["url"], req.get("cookies"))
        res.update({"rid": req["rid"], "ok": True})
        send(res)
    except Exception as e:  # noqa: BLE001
        send({"rid": req["rid"], "ok": False, "error": str(e)})


def warm():
    # İlk "şimdi çal" isteği soğuk başlamasın: örneği (çerezlerle) önceden kur.
    try:
        y = ydl(True, WARM_COOKIES)
        _ = y.cookiejar  # çerezleri şimdi yükle (yt-dlp tembel yükler)
    except Exception:  # noqa: BLE001
        pass


def worker():
    warm()
    while True:
        _prio, _n, req = jobs.get()
        handle(req)


def urgent_worker():
    warm()
    while True:
        handle(urgent.get())


def main():
    global seq
    threading.Thread(target=urgent_worker, daemon=True).start()
    for _ in range(2):
        threading.Thread(target=worker, daemon=True).start()
    send({"ready": True})
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            req = json.loads(line)
        except ValueError:
            continue
        seq += 1
        if int(req.get("prio", 1)) == 0:
            urgent.put(req)
        else:
            jobs.put((int(req.get("prio", 1)), seq, req))


if __name__ == "__main__":
    main()
