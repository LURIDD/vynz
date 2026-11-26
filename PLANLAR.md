# Vynz — Geliştirme Planları ve Değerlendirme (22 Kas 2026)

Bu belge, kullanıcının (LURID) bildirdiği 12 geliştirme/sorun maddesini, teknik kök neden analizlerini ve **Gemini (Neon)** ile **Claude (Noema)** ortak görüş/eleştirilerini içerir.

---

## Önerilen Uygulama Sırası

1. **Çözücü hızı & 8 sn bekleme düzeltmesi** *(en yüksek öncelik — doğrudan hissedilen darboğaz)*
2. **Kuyrukta sağ tık menüsü + Chip dinamik boyut düzeltmesi** *(hızlı UI düzeltmeleri)*
3. **Discord RPC** *(bağımsız, düşük riskli)*
4. **Kişiselleştirme & Algoritma yenilenmesi** *(TR trend hakimiyetini kırma, "Bunu daha çok öner", algoritma sıfırlama, zevke göre topluluk listeleri)*
5. **Lyrics sayfa düzeni** *(büyük/merkeze yakın kapak, tam ekranda alt bar gizleme, kapağın altına basit kontroller, hover + kalıcı beğeni)*
6. **Görsel RAM optimizasyonu** *(gerçek görüntü boyutuna ölçekleme + afiş bellek serbest bırakma)*
7. **Arama sayfası düzeni** *(geniş/dar pencereye duyarlı bölünmüş düzen)*
8. **Listelerim sayfası görsel yenilenmesi** *(2×2 kapak mozaiği, kapaktan renk geçişli üst başlık, toplam süre, yeniden adlandırma & liste içi filtre)*

---

## Maddeler, Kök Nedenler ve Yorumlar

### 1. Çözücü yavaşlığı & sanatçıya/yeni şeye tıklayınca ~8 sn bekleme
- **Kök neden (kodda doğrulandı):** `src/main.rs` içinde "şimdi çal" akış çözmesi, `radio`, `search` ve `collection` (sanatçı/albüm) isteklerinin **tamamı `Prio::Now` (0)** önceliğiyle gidiyor ve `src/resolver.py` içindeki **tek** `urgent_worker` iş parçacığında sıraya giriyor. Bir şarkıya tıklandığında ağır `yt-dlp` çözmesi + radyo o tek iş parçacığını 5–8 sn tıkıyor; o sırada tıklanan sanatçı sayfası arkada bekliyor.
- **Ek yavaşlık noktası:** `resolver.py` başlatılırken 3 worker'ın tamamı (`urgent_worker` + 2 `worker`) `warm()` ile `yt-dlp` + çerez yüklüyor; ayrıca ilk açılışta arka planda çok sayıda ön-çözme (`speculate`) işi birikiyor.
- **Çözüm:**
  - `resolver.py`'de **ayrı bir `meta` iş parçacığı havuzu (3 thread)** açılacak; `op` alanı `search`, `collection`, `radio`, `lyrics`, `home`, `explore` olan tüm hafif `ytmusicapi` HTTP çağrıları bu havuzda anında çalışacak (`warm()` ile `yt-dlp` yüklemeyecek!).
  - `urgent_worker` **yalnızca** "Şimdi çal" `yt-dlp` ses akışı çözmesine ayrılacak.
  - **Beklenen etki:** Sanatçı/albüm/arama tıklamaları ~8 sn'den **<0.5 sn** seviyesine düşer; şarkı çözmesi de meta istekler tarafından bloklanmaz.

### 2. Kuyruktaki müziklere sağ tık menüsü açılmıyor
- **Kök neden:** `ui/app.slint` kuyruk çekmecesindeki `qi-ta := TouchArea` yalnızca `clicked` dinliyor, `pointer-event` ile sağ tuş yakalamıyor (`menu-target` zaten `kind == 3` kuyruk desteğine sahip!).
- **Çözüm:** Sağ tık eklenip kuyruğa özel aksiyonlar (*Sıradaki yap, Kuyruktan çıkar, Radyo başlat, Beğen, Sanatçıya git, İlgilenmiyorum, Bunu daha çok öner*) bağlanacak.

### 3. Chip butonlarda dinamik boyut / sağ kenara yapışma (`Kuyruk` ve `Kaldığı yere dön`)
- **Kök neden:** `HorizontalLayout` içine `x: 14px` verilince `HorizontalLayout` parent genişliğini miras alıp sağdan 14px taşıyor.
- **Çözüm:** `x: 14px` kaldırılıp `padding-left: 14px; padding-right: 14px;` (veya `width: row.preferred-width`) ile %100 simetrik iç boşluk verilecek.

### 4. Discord RPC desteği
- **Değerlendirme:** Düşük riskli, yüksek keyif veren bir özellik. `discord-rich-presence` crate'i ile çalan şarkı adı, sanatçı, süre (kalan/geçen zaman çubuğu) ve YouTube Music kapak URL'si doğrudan Discord'a yansıtılabilir.
- **Detay:** Ayarlar → Çalma & Ses (veya Kaynaklar) altında aç/kapa anahtarı olacak; Discord kapalıyken sessizce bekleyip açıldığında otomatik bağlanacak.

### 5. Ana sayfanın TR trendleriyle dolu olması, "Sana özel"in özel hissettirmemesi & Algoritma geliştirme
- **Kök neden:**
  1. `resolver.py` içindeki `YTMusic(language="tr", location="TR")` girişsiz çağrıldığı için `home()` ve `explore()` tamamen anonim **Türkiye trend vitrini** döndürüyor.
  2. `compose()` içinde bu harici TR trend rafları (*Trend şarkılar*, *Türkiye'de popüler sanatçılar*, *Yeni albümler*, harici *Hızlı seçimler* ve 2 adet TR topluluk listesi) hâlâ ana sayfanın yarısından fazlasını kaplıyor.
  3. Zevkler seçilse bile `seeds_with_tastes` toplam en çok 5 tohum alıyor ve harici TR raflarını filtrelemiyor.
- **Çözüm:**
  - **TR trendlerini arka plana it / gizle:** Kullanıcının geçmişi veya Zevkleri varsa ana sayfanın ilk %80'i **tamamen yerel profil + Zevkler + benzer sanatçı radyolarından** üretilecek; genel TR trendleri en altta küçük/isteğe bağlı bir "Keşfet" alanına inecek (veya tamamen Zevkler'e göre filtrelenecek).
  - **"Sana özel" gerçekten özel olsun:** Beğenilen + yüksek puanlı + *"Bunu daha çok öner"* işaretli şarkıların radyolarından, zevk profiliyle örtüşen yüksek skorlu özel karışımlar üretilecek (ör. *Senin için karışım*, *Gece/Ruh hâli karışımı*, *Yeni keşifler*).
  - **Çoklu tohum + sanatçı grafı:** Yalnızca 5 değil, geçmiş + Zevkler + *"Daha çok öner"* sinyallerinden dinamik tohum havuzu ve harici rafları kullanıcının profiliyle yeniden puanlama.

### 6. "Bunu daha çok öner" seçeneği
- **Tasarım:** Mevcut *İlgilenmiyorum* (`dislikes.json`) mekanizmasının artı (+) karşılığı (`boosted_tracks` / `boosted_artists` veya `boosts.json`).
- **Etkisi:** Sağ tık menüsünde `↑ Bunu daha çok öner` seçildiğinde şarkı ve sanatçısına güçlü pozitif ağırlık (+4.0) verilir, tohum seçiminde öncelik kazanır; Ayarlar → Zevkler'den yönetilebilir.

### 7. Algoritma sıfırlama
- **Noema'nın kritik uyarısı:** Tek tuşla her şeyi silmek kullanıcının dinleme geçmişini/istatistiklerini kaybetmesine yol açabilir.
- **Çözüm:** Ayarlar → Zevkler altında iki net seçenek:
  1. **Öneri profilini sıfırla** *(Zevkler, "Daha çok öner", "İlgilenmiyorum" ve ana sayfa önbelleğini sıfırlar; dinleme geçmişin ve beğenilerin korunur)*.
  2. **Dinleme geçmişini de temizle** *(tamamen sıfırdan başla — onaylı)*.

### 8. "Trend olan topluluk oynatma listeleri" yerine zevke/algoritmaya göre topluluk listeleri
- **Tasarım:** Anonim TR trend listeleri yerine, kullanıcının en çok dinlediği 2–3 sanatçı + seçili Zevk türleri için `ytmusic().search(q, filter="playlists")` ile gerçek topluluk çalma listeleri çekilip *"Zevkine göre çalma listeleri"* rafında sunulacak (önbellekli, arka planda hızlı).

### 9. Lyrics sayfası düzeni
- **Kullanıcı fikri:** Kapağı duvardan biraz daha merkeze yaklaştırıp büyütmek; Lyrics tam ekrandayken alt oynatma çubuğunu gizleyip basit kontrolleri kapağın altına almak; beğenme tuşunu kapağın üstüne hover'da eklemek.
- **Ortak değerlendirme (Gemini + Noema):**
  - Çok şık ve sinematik (Apple Music / Tidal tarzı)! Sol sütundaki kapak ~340–360px'e büyütülüp sola/merkeze dengeli yaslanabilir.
  - Alt çubuk Lyrics açıkken yumuşakça aşağı kayıp gizlenir; kapağın altında sade bir progress bar + `⏮ ▶/⏸ ⏭` + ses/kuyruk mini kontrolleri yer alır.
  - **Önemli UX uyarısı (Noema + Gemini):** Beğenme (kalp) tuşu *yalnızca* hover'da çıkarsa bulunması zorlaşır ve zaten beğendiğin bir şarkıda durumu göremezsin. **En iyi orta yol:** Şarkı zaten beğenilmişse kapağın köşesinde ince bir kalp rozeti hep görünür; beğenilmemişse hover'da belirir (veya kapağın altındaki başlık satırının sağında da sade bir kalp durur).

### 10. Arama sonuçları sayfası: dikey ikiye bölünmüş düzen tartışması
- **Kullanıcı fikri:** Sol tarafta albümler/sanatçılar/listeler, sağ tarafta aşağıya doğru şarkı listesi.
- **Eleştiri (Noema + Gemini):**
  - **Avantajı:** Şarkılara erişmek için albüm/sanatçı/liste raflarını aşağı kaydırmak zorunda kalmıyorsun — mevcut sayfadaki en büyük sürtünmeyi çözüyor.
  - **Riski:** 900–1100 px gibi dar pencerelerde 2 eşit sütun hem şarkı başlıklarını keser hem albüm kartlarını sıkıştırır.
  - **Önerilen en iyi tasarım:**
    - **Geniş pencerede (`>= 1150px`):** İki sütunlu asimetrik düzen — **Sağda (%58–60)** en üstte *En iyi sonuç* + hemen altında kesintisiz aşağı uzanan **Şarkılar** listesi; **Solda (%40–42)** kompakt 2'li kartlar hâlinde *Sanatçılar*, *Albümler*, *Oynatma listeleri* (veya üstte sekmeli: *Tümü · Şarkılar · Albümler · Sanatçılar · Listeler*).
    - **Dar pencerede (`< 1150px`):** Şarkılar en üstte, albüm/sanatçı/listeler altında (böylece şarkıya ulaşmak dar ekranda da zahmetsiz olur).

### 11. Görsel RAM tasarrufu (önbellekteki görselleri boyutlarına göre küçültme/sıkıştırma)
- **Teknik gerçek (Gemini + Noema):**
  - Diskteki JPEG/PNG dosyasını sıkıştırmak **RAM'i azaltmaz**, çünkü Slint ekranda göstermek için görseli açılmış ham **RGBA (piksel başına 4 bayt)** olarak RAM'de (ve GPU dokusunda) tutar.
  - **Gerçek RAM kazanımı sağlayacak 4 adım:**
    1. **Gerçek gösterim boyutuna ölçekleme:** Şu an küçük kapaklar `art::SMALL = 192` (192×192×4 ≈ 147 KB/kapak) çözülüyor; oysa sol bar, hızlı erişim ve liste satırlarında 38–48 px, kartlarda ~140–170 px görünüyor. `SMALL`'ı `128`'e indirmek kapak RAM'ini anında **%55 azaltır** (147 KB → 65 KB), gözle görülür kalite kaybı olmadan.
    2. **Afiş (`1440×600`, ~3.4 MB) yaşam ömrü:** Sanatçı sayfasından çıkıldığında afişi RAM'den hemen serbest bırakmak + 960×400 (~1.5 MB) boyutunda çözmek.
    3. **Kopya bellek kontrolü:** `art::Art` içindeki `rgba` vektörü Slint `Image` oluşturulduktan sonra hemen drop ediliyor mu doğrulanacak; LRU sınırları sıkı tutulacak.
    4. **Tepsi düşük RAM modu (zaten eklendi):** Pencere gizliyken tüm görsel tamponlar boşaltılıyor.

### 12. Listelerim sayfası görsel yenilenmesi
- **Sorun:** Kayıtlı bir listeyi (veya Beğendiklerim'i) açınca üstte yalnızca düz metin başlık + şarkı sayısı + `Çal`/`Sil` düğmeleri var; grafiksel olarak tekdüze duruyor.
- **Tasarım önerisi:**
  - **2×2 Kapak Mozaiği (kolaj):** Listede 4+ farklı kapak varsa 180×180 yuvarlatılmış **2×2 kapak kolajı** (1–3 şarkıysa tek büyük kapak, boşsa Vynz degrade yer tutucu) — hem liste sayfasında hem sol bardaki liste küçük resminde çok daha canlı durur.
  - **Kapaktan türeyen üst zemin geçişi:** Liste üst başlığının arkasında ilk kapaktan gelen yumuşak renk geçişi (`@linear-gradient`) + toplam süre (`24 şarkı · 1 sa 18 dk`) + `Çal`, `Karıştır`, `Yeniden adlandır`, `Sil` aksiyonları.
  - **Liste içi hızlı filtre & düzenleme:** Uzun listelerde anında şarkı bulmak için küçük bir arama/filtre kutusu ve şarkıları listeden tek tıkla çıkarma / sırasını değiştirme.


---

## Fix listesi (bekleyen hatalar)

### F1. Söz vitrininde ışımanın kutu gibi kesilmesi (24 Kas 2026)
- **Görünüm:** Müzik arasında etkin satır `♪` olduğunda arkasındaki ışıma yuvarlak sönmüyor, sert kenarlı bir kare/dikdörtgen olarak kesik görünüyor. Ekran görüntüsü: `notes/fix/isik-kopmasi.jpg` (SONBAHAR, 00:05, Modern stil).
- **İlgili kod:** `ui/parts.slint` `KaraokeWord` → `glow := Rectangle` (kelimeden `size * 0.9` taşan kare, radial degrade).
- **Neden:** Kare ışıma kelimenin solundan taşıyor (dar `♪`'da ~1 fsz), `lview` (`clip: true`) sol kenarında kesiliyordu.
- **Durum:** DÜZELTİLDİ (24 Kas). `lview`'a iki yana `gp = min(1.1·fsz, boşluk-12px)` iç pay; metin aynı yerde, ışık kırpılmadan söner (piksel ölçümüyle doğrulandı).

### F2. Söz kipinde sol barın altında boşluk (24 Kas 2026)
- **Görünüm:** Sözler tam ekran açıkken (alt oynatma çubuğu gizli) sol bar açılınca `Ayarlar`'ın altında ~110 px boş alan kalıyor; bar pencerenin altına kadar uzanıyor ama içerik alt çubuk hâlâ varmış gibi yukarıda bitiyor. Ekran görüntüsü: `notes/fix/sol-bar-alt-bosluk.png`.
- **Neden:** Sol bar `padding-bottom: Theme.player-clearance` (116 px) söz kipinde de kalıyordu.
- **Not (aynı görüntü):** Sol bar söz kipinde kapağın ve kontrollerin üstüne biniyor (kapak, başlık, `-0.5 sn` soldan kesik). İstenen davranışsa sorun değil; değilse içerik bar kadar sağa kaymalı (açık).
- **Durum:** Boşluk DÜZELTİLDİ (24 Kas): söz kipinde alt pay 16 px.

### F3. YTM listesi aktarılırken bazı şarkıların boş profille gelmesi (25 Kas 2026)
- **Görünüm:** YouTube Music'ten çekilen bir listede bazı şarkılar boş profil içinde (başlık/sanatçı/kapak eksik) geliyor; nedeni bilinmiyor.
- **İncelenecek liste:** https://music.youtube.com/playlist?list=PLsH3S5xRu5wPodftftgUgslBjYDDPAwsn
- **Bakılacak yer:** `src/import.rs` (yt-dlp flat YTM aktarımı); flat çıktıda eksik alan gelen öğeler (kullanılamayan/bölgesel kısıtlı video, video vs. şarkı ayrımı, sanatçı alanı boş) olabilir. **Tahmin**, doğrulanmadı.
- **Durum:** Açık.

### F4. Mini oynatıcıda beğeni kalbinin yalnız kenarlığının değişmesi (25 Kas 2026)
- **Görünüm:** Mini oynatıcıda şarkı beğenilince kalbin içi dolmuyor, yalnız kenarlık rengi değişiyor; beğenilip beğenilmediği anlaşılmıyor.
- **İstenen:** Beğenilince kalbin içi dolu olmalı (ana çubuktaki davranışla aynı).
- **Durum:** Açık.

---

## Yeni özellikler

### Y1. Windows için cava benzeri görselleştirici (25 Kas 2026)
- Linux'ta görselleştirici ve "müziğe tepki" `cava` ham çıktısına dayanıyor (`src/viz.rs`); Windows'ta cava yok.
- Windows karşılığı gerekli: örn. WASAPI loopback ile ses yakalayıp FFT'yi Rust içinde yapmak (cava'ya bağımlılığı tamamen kaldırma seçeneği de var).
- **Durum:** Fikir aşamasında.

### F5. Discord durumunun bazen "Dinliyor" yerine "Oynuyor"a düşmesi (26 Kas 2026)
- **Görünüm:** Vynz çalarken Discord durumu zaman zaman "… dinliyor" yerine "… oynuyor" gösteriyor; ne zaman olduğu belli değil.
- **İlgili kod:** `src/discord.rs` gönderim döngüsü (~satır 153-161) ve `activity()` (~satır 213-231).
- **Güçlü aday (Tahmin, doğrulanmadı):** `set_activity` herhangi bir nedenle reddedilince (çok uzun başlık, geçersiz kapak adresi, boş alan vb.) kod bunu "eski istemci `type` alanını tanımıyor" sayıp `no_type = true` yapıyor ve bu bayrak oturum boyunca hiç sıfırlanmıyor. Sonraki tüm durumlar `type: 2` (Listening) olmadan gidiyor, Discord da varsayılan olarak "Oynuyor" gösteriyor.
- **Düzeltme yönü:** Reddin nedenini Discord'un hata yanıtından okuyup yalnız `type` hatasında düşmek; ya da `no_type`'ı şarkı değişiminde/yeniden bağlantıda sıfırlamak. Reddedilen içeriği (hangi şarkıda olduğu) günlüğe yazmak.
- **Durum:** Açık.
