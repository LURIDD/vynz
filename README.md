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
- **Ses:** 10 bantlı ekolayzır (hazır ayarlar), şarkılar arası çapraz geçiş,
  ses seviyesi dengeleme.
- **İndirilenler:** Şarkıları ya da listelerin tamamını indirip internet
  olmadan dinleme.
- **Görünüm:** Kapaktan otomatik ya da sabit vurgu rengi, beş koyu zemin.
- **Kendini güncelleme:** Yeni sürüm çıkınca açılışta küçük bir pencerede
  indirilir, doğrulanır ve uygulama yeniden açılır; GitHub'dan elle indirmek
  gerekmez.
- **Masaüstü uyumu:** Linux'ta MPRIS (medya tuşları, `playerctl`), Windows'ta
  SMTC (medya tuşları ve sistem medya paneli), kaldığı yerden devam, kotalı
  bağlantı ve pil farkındalığı.

## Kurulum

Hazır derlemeler [Releases](https://github.com/LURIDD/vynz/releases) sayfasında
paylaşılır:

- **Windows (x86_64), kurulum:** `vynz-windows-x86_64-setup.exe`'yi çalıştırın.
  Yönetici izni istemez, yalnız sizin kullanıcınıza kurulur.
- **Windows (x86_64), taşınabilir:** `vynz-windows-x86_64.zip` arşivini bir klasöre açıp
  `vynz.exe`'yi çalıştırın (`libmpv-2.dll` pakete dahildir). İlk açılışta
  çözücü ortamı `%LOCALAPPDATA%\vynz\runtime` klasörüne kendiliğinden kurulur.
- **Linux (x86_64):** `vynz-linux-x86_64.tar.gz` arşivini açıp `vynz` ikilisini
  çalıştırın. Sistemde `libmpv` kurulu olmalıdır (Arch: `mpv`, Debian/Ubuntu:
  `libmpv2`).

### Windows SmartScreen uyarısı

Vynz'nin Windows dosyaları henüz kod imzalı değildir (bkz.
[Code signing policy](#code-signing-policy)). Bu yüzden Windows ilk açılışta
"Windows kişisel bilgisayarınızı korudu" uyarısı gösterebilir: **Ek bilgi →
Yine de çalıştır** ile devam edebilirsiniz. İndirdiğiniz dosyanın sürüm
sayfasındakiyle aynı olduğunu `SHA256SUMS` ile doğrulayabilirsiniz:

```powershell
Get-FileHash .\vynz-windows-x86_64-setup.exe -Algorithm SHA256
```

### Sürümler

Sürümler `0.MINOR.PATCH` biçimindedir: yeni özellikler `MINOR`'ı (0.3.0),
yalnız düzeltmeler `PATCH`'i (0.2.1) artırır. Tire içeren etiketler
(`v0.3.0-beta.1`) ön sürümdür ve uygulama içi güncelleyiciye görünmez.

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

## Code signing policy

Windows sürümleri GitHub Actions üzerinde bu depodan derlenir ve imzalanır;
ayrıntılar, ekip rolleri ve gizlilik bilgisi
[CODE_SIGNING_POLICY.md](CODE_SIGNING_POLICY.md) dosyasındadır.

Free code signing provided by [SignPath.io](https://about.signpath.io),
certificate by [SignPath Foundation](https://signpath.org) *(başvuru
aşamasında; şimdiki sürümler imzasızdır)*.

## Lisans

Kod [MIT](LICENSE) lisanslıdır. `ui/fonts` içindeki yazı tipleri
[SIL Open Font License 1.1](ui/fonts/OFL.txt) ile dağıtılır.
