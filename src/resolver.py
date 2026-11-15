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
import queue
import sys
import threading
import time
import urllib.parse

import yt_dlp

FAST_ARGS = {"youtube": {"player_client": ["web_music"], "player_skip": ["webpage", "configs"]}}

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
        a = y.get_artist(key)
        songs = a.get("songs") or {}
        res = []
        if songs.get("browseId"):
            try:
                res = y.get_playlist(songs["browseId"], limit=60).get("tracks") or []
            except Exception:  # noqa: BLE001
                res = []
        if not res:
            res = songs.get("results") or []
        return {"title": a.get("name") or "", "sub": "Sanatçının en sevilen şarkıları", "tracks": [x for x in (track_of(t) for t in res) if x]}
    if kind == "mood":
        pls = y.get_mood_playlists(key)
        cards = [{"kind": "playlist", "key": p["playlistId"], "title": p.get("title") or "", "sub": p.get("description") or "", "thumb": thumb_of(p)} for p in pls if p.get("playlistId")]
        return {"title": "", "sub": "", "cards": cards}
    p = y.get_playlist(key, limit=200)
    return {"title": p.get("title") or "", "sub": (p.get("author") or {}).get("name") if isinstance(p.get("author"), dict) else "", "tracks": [x for x in (track_of(t) for t in p.get("tracks") or []) if x]}


def search(q):
    y = ytmusic()
    res = y.search(q, limit=20)
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


def handle(req):
    op = req.get("op")
    if op == "search":
        try:
            send({"rid": req["rid"], "ok": True, "data": search(req.get("q") or "")})
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
