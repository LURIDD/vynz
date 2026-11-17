# Vynz

Rust ve Slint ile yazılmış, hafif ve çok kaynaklı bir masaüstü müzik çalar.
YouTube Music ve SoundCloud'dan çalar, Spotify listelerini içe aktarır, zamanlı
sözleri karaoke gibi gösterir.

*A lightweight multi-source desktop music player written in Rust + Slint:
plays from YouTube Music and SoundCloud, imports Spotify playlists, and shows
time-synced karaoke lyrics. The UI is in Turkish.*

## Özellikler

- **Çalma:** libmpv ile; sıradaki şarkıyı önceden hazırlar (boşluksuz geçiş),
  dinlenen şarkıları hız sınırlı olarak disk önbelleğine alır.
- **Ana sayfa:** dinleme geçmişine göre yerel öneri algoritması, radyo, ruh hâli
  ve türler, sanatçı ve albüm sayfaları.
- **Sözler:** zamanlı satırlar, kelime kelime dolum, isteğe bağlı çeviri,
  büyük kapaklı "Şimdi Çalıyor" sahnesi.
- **Liste aktarma:** Spotify (bağlantı, veri dışa aktarımı JSON'u), YouTube Music,
  SoundCloud, CSV.
- **Masaüstü uyumu:** MPRIS (medya tuşları, playerctl), kaldığı yerden devam.

## Gereksinimler (Linux)

- Rust (2024 sürümü), `libmpv`
- `python3` ile [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) ve
  [`ytmusicapi`](https://github.com/sigma67/ytmusicapi)
- İsteğe bağlı: `cava` (profil ekranındaki spektrum), `zenity` ya da `kdialog`
  (dosyadan liste aktarma), `nmcli` (kotalı bağlantıda önbelleği durdurma)

```sh
cargo build --release
./target/release/vynz
```

Şu an yalnız Linux'ta geliştirilip denenmektedir.

## Söz kaynakları

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
