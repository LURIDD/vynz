# vynz çözücü yardımcısı: sayfa adresinden doğrudan ses akışı adresi.
# Rust tarafı başlatır, stdin/stdout üzerinden JSON satırlarıyla konuşur ve
# boşta kalınca kapatır. Kullanıcının ~/.config/yt-dlp/config dosyası okunmaz
# (YoutubeDL API'si ayar dosyası okumaz).
#
# İstek : {"rid": 1, "url": "...", "prio": 0, "cookies": "firefox"|null}
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


def handle(req):
    try:
        res = resolve(req["url"], req.get("cookies"))
        res.update({"rid": req["rid"], "ok": True})
        send(res)
    except Exception as e:  # noqa: BLE001
        send({"rid": req["rid"], "ok": False, "error": str(e)})


def worker():
    while True:
        _prio, _n, req = jobs.get()
        handle(req)


def urgent_worker():
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
