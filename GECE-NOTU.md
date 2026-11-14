# Gece notu — 15 Kasım 2026

Günaydın! Gece yapılanların özeti. Her şey commit'li (`git log --oneline`).

## Nasıl açılır
- Uygulama menüsünde **Vynz**, ya da terminalde `vynz`.
- `~/.local/bin/vynz` → `~/Projeler/vynz/target/release/vynz` bağlantısı. Kodu değiştirince `cargo build --release` yeterli.

## 1. Hız: şarkılar artık çok daha hızlı açılıyor
Ölçülen tıklama → ilk ses süreleri (önce hepsi 9–10 sn idi):

| Durum | Süre |
|---|---|
| Hiç hazırlanmamış şarkı | ~4.4 sn |
| Önceden hazırlanmış şarkı (sıradakiler, ekrandaki ilk şarkılar, üzerinde beklenen şarkı) | ~0.3 sn |
| Sıradakine geçiş (şarkı bitince ya da "sonraki") | ~0.003 sn, boşluksuz |
| Daha önce dinlenmiş şarkı (disk önbelleği) | ~0.03 sn |

Nasıl:
- **Çözücü yardımcısı** (`src/resolver.py`): yt-dlp bellekte açık kalan tek bir işlem; senin `~/.config/yt-dlp/config` dosyanı okumaz, tek istemci kullanır. Yalnız iş varken açık; 5 dk boşta kalınca kapanır (çalışırken ~69 MB).
- **Ön-çözme**: sıradaki 2 şarkı + ekrandaki ilk şarkılar + üzerinde 0.45 sn beklenen şarkı. Pilde ve kotalı bağlantıda yalnız sıradakiler.
- **Ön-tampon**: mpv sıradaki şarkıyı, çalanın verisi bitince açmaya başlar (önce çalan gelir).
- **Disk önbelleği**: 20 sn'den uzun dinlenen şarkı arka planda indirilir. Hız sınırı var (varsayılan 512 KB/sn), başka program interneti yoğun kullanıyorsa bekler, kotalı bağlantıda indirmez. Boyut Ayarlar'dan (varsayılan 2 GB).
- **rustypipe denendi, olmadı**: girişsiz yalnız ilk 1 MB veriyor, PO jetonuyla da güncel YouTube kodunu çözemiyor (proje 2025 ortasından beri güncellenmiyor). Kendi Rust çözücümüz (yt-dlp'nin çözücü betiğiyle) sonraki adım olarak duruyor.

## 2. Ayarlar ekranı
Önbellek boyutu (Kapalı/1/2/5/10 GB) + Temizle, arka plan indirme hızı, önceden hazırlama aç/kapa, ileri okuma (internet kullanımı), YouTube Music girişi (Firefox oturumu → Premium'da ~260 kbps).

## 3. Liste aktarma (Spotify / YouTube Music / SoundCloud)
- Bağlantıyı yapıştır → Aktar. Spotify şarkıları YouTube Music'te (bulunamazsa SoundCloud'da) eşleştirilir.
- İnceleme ekranı: Eşleşti / Emin değil / Bulunamadı, "Değiştir" ile başka aday seçilir.
- "Kütüphaneye kaydet" → kenar çubuğunda **Listelerim** altında; aç, çal, sil.
- Gerçek testler: Today's Top Hits 50/50, Top 50 Türkiye 50/50, All Out 80s 100/100.
- Sınır: Spotify'ın gömülü sayfası en fazla **100 şarkı** veriyor ve yalnız **herkese açık** listeler. Daha fazlası için Spotify veri dışa aktarımı desteği sonraki iş.

## 4. Söz Vitrini
Kelime kelime dolan büyük satır, önceki/sonraki satırlar, bulanık kapak zemini, şarkıya özel ±0.5 sn kaydırma. Kaynak: YouTube Music'in kendi sözleri (ymus), yoksa LRCLIB.

## 5. MPRIS
Quickshell çubuğundaki medya penceresi ve klavyedeki medya tuşları Vynz'i yönetir (`playerctl` ile denendi; gerçek tuşa basılmadı).

## Bilmen gerekenler
- **ymus OAuth**: `ymus yt playlist` komutu `KeyError: 'access_token'` veriyor. Büyük ihtimalle Google projesinin "Testing" modundaki 7 günlük jeton süresi doldu. Vynz'in aktarması bunu kullanmıyor (yt-dlp ile okuyor), ama ymus'un hesap komutları etkilenir. Kalıcı çözüm: Google Cloud'da uygulamayı "Production"a almak.
- **Bellek**: optimize sürüm boşta ~190 MB (programın kendine ait kısmı ~63 MB; kalanı mpv ve sürücü kitaplıkları). Çalarken ~%9 işlemci; büyük kısmı ekolayzer çubuklarının sürekli çizimi, azaltılabilir.
- Testler sanal masaüstünde, sesi kapalı (`VYNZ_AO=null`) yapıldı. Hoparlörden dinleyerek deneme senin.
- Hız ölçümü için: `VYNZ_DEBUG=1 vynz` zaman damgalı günlük yazar.

## Sıradaki işler (öneri)
1. Kendi YouTube çözücümüz (soğuk açılışı ~1–2 sn'ye indirmek için).
2. Spotify veri dışa aktarımı / CSV ile 100'den uzun ve gizli listeler.
3. Karıştır, tekrar, beğen düğmeleri (şu an pasif).
4. Dinleme odaları, profil ve görselleştirici.
5. Ekolayzer çubuklarının işlemci yükünü azaltmak.
