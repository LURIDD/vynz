# Vynz

Rust ve Slint ile yazılmış, hafif ve çok kaynaklı bir masaüstü müzik çalar.
YouTube Music ve SoundCloud'dan çalar, Spotify listelerini içe aktarır, zamanlı
sözleri karaoke gibi gösterir. Linux ve Windows destekler.

*A lightweight multi-source desktop music player written in Rust + Slint for
Linux and Windows: plays from YouTube Music and SoundCloud, imports Spotify
playlists, and shows time-synced karaoke lyrics. The UI is in Turkish.*

## Özellikler

- **Çalma:** `libmpv` ile; sıradaki şarkıyı önceden hazırlar (boşluksuz geçiş),
  dinlenen şarkıları hız sınırlı olarak disk önbelleğine alır.
- **Kendi kendine yeten çözücü:** İlk açılışta Vynz veri klasörüne izole `uv` +
  Python + [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) +
  [`ytmusicapi`](https://github.com/sigma67/ytmusicapi) + `deno` ortamını kurar;
  sistem Python'una dokunmaz ve `yt-dlp`'yi birkaç günde bir sessizce günceller.
- **Ana sayfa:** Dinleme geçmişine göre yerel öneri algoritması, radyo, ruh hâli
  ve türler, sanatçı ve albüm sayfaları.
- **Sözler:** Zamanlı satırlar, kelime kelime dolum, isteğe bağlı çeviri,
  büyük kapaklı "Şimdi Çalıyor" sahnesi.
- **Liste aktarma:** Spotify (bağlantı, veri dışa aktarımı JSON'u), YouTube Music,
  SoundCloud, CSV (yerleşik sistem dosya seçicisi ile).
- **Masaüstü uyumu:** Linux'ta MPRIS (medya tuşları, `playerctl`), Windows'ta
  SMTC (medya tuşları ve sistem medya paneli), kaldığı yerden devam, kotalı
  bağlantı ve pil farkındalığı.

## Kurulum

Hazır derlemeler [Releases](https://github.com/LURIDD/vynz/releases) sayfasında
paylaşılır:

- **Windows (x86_64):** `vynz-windows-x86_64.zip` arşivini bir klasöre açıp
  `vynz.exe`'yi çalıştırın (`libmpv-2.dll` pakete dahildir). İlk açılışta
  çözücü ortamı `%LOCALAPPDATA%\vynz\runtime` klasörüne kendiliğinden kurulur.
- **Linux (x86_64):** `vynz-linux-x86_64.tar.gz` arşivini açıp `vynz` ikilisini
  çalıştırın. Sistemde `libmpv` kurulu olmalıdır (Arch: `mpv`, Debian/Ubuntu:
  `libmpv2`).

## Kaynaktan derleme

### Linux

- Rust (2024 sürümü) ve `libmpv` geliştirme başlıkları (`mpv` / `libmpv-dev`)
- İsteğe bağlı: `cava` (profil ekranındaki canlı spektrum), `nmcli` (kotalı
  bağlantıda arka plan önbelleğini durdurma)

```sh
cargo build --release
./target/release/vynz
```

### Windows

- Rust (MSVC) ve `libmpv-2.dll` + `mpv.lib` (bkz.
  [`.github/workflows/build.yml`](.github/workflows/build.yml))

## Söz kaynakları

- [Unison](https://unison.betterlyrics.org): Better Lyrics topluluğunun söz
  veritabanı. Önce sorulur; hece zamanlı (TTML) sözlerde karaoke dolumu gerçek
  zamanlarla yapılır. Sözler, şarkı sayfasına bağlı **Lyrics from Unison**
  yazısı ve sözü gönderen kişinin adıyla gösterilir. Unison verisi
  [Unison Community License 1.0](https://github.com/better-lyrics/unison/blob/master/LICENSE-DATA.md)
  ile kullanılır: ücretsizdir ve yalnız kişisel, kâr amacı gütmeyen ya da açık
  kaynak kullanım içindir. İndirilen sözler ETag ile yerelde önbelleğe alınır,
  Unison'dan kaldırılan sözler önbellekten de silinir.
- YouTube Music
- [LRCLIB](https://lrclib.net)

## Not

Vynz kişisel kullanım için geliştirilmiş, bağımsız ve ücretsiz bir projedir;
YouTube, SoundCloud ya da Spotify ile bağlantısı yoktur. Bu servislerin kendi
kullanım koşulları geçerlidir. Şarkı sözlerinin hakları yazarlarına ve
yayıncılarına aittir.

## Lisans

Kod [MIT](LICENSE) lisanslıdır. `ui/fonts` içindeki yazı tipleri
[SIL Open Font License 1.1](ui/fonts/OFL.txt) ile dağıtılır.
