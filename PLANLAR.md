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
- **Bakılacak yer:** `src/import.rs` `flat_list` (yt-dlp flat YTM aktarımı).
- **Kök neden (27 Kas, DOĞRULANDI):** Yaş kısıtlaması DEĞİL. 49 şarkılık listede 4 öğe yalnız `id` ile geliyor (başlık/kanal/süre `null`); dördü de YouTube'dan silinmiş: `HRaYiVprRV8`, `lrPVfFYQtJc`, `kg5lkuA_1BU` telif şikâyetiyle (Andina Music LLC), `daSm4RUaxcI` kanalı kapatıldığı için. `flat_list` başlıksız öğeyi boş başlık/sanatçıyla listeye ekliyor.
- **Düzeltme:** Başlığı `null` öğeleri aktarmadan çıkar; aktarım sonunda "N şarkı YouTube'da artık yok" bildir.
- **Durum:** DÜZELTİLDİ (27 Kas): başlıksız öğeler atlanıyor, aktarım notunda "N şarkı kaynakta artık yok, atlandı".

### F4. Mini oynatıcıda beğeni kalbinin yalnız kenarlığının değişmesi (25 Kas 2026)
- **Görünüm:** Mini oynatıcıda şarkı beğenilince kalbin içi dolmuyor, yalnız kenarlık rengi değişiyor; beğenilip beğenilmediği anlaşılmıyor.
- **İstenen:** Beğenilince kalbin içi dolu olmalı (ana çubuktaki davranışla aynı).
- **Durum:** DÜZELTİLDİ (27 Kas): mini oynatıcıda `filled: root.liked` (ekran görüntüsüyle doğrulandı).

### F7. Kuyrukta "Sanatçıyı önerme" / "İlgilenmiyorum" şarkıyı atlamıyor ve kuyruktan silmiyor (27 Kas 2026)
- **Bildiren:** kullanıcının arkadaşı (Discord).
- **Neden:** İki eylem yalnız tercihi kaydediyordu. "İlgilenmiyorum" yalnız çalan şarkıya uygulanınca atlıyordu; kuyruktaki istenmeyen şarkılar kuyrukta kalıyordu, atlanınca sıradaki yine aynı sanatçı olabiliyordu.
- **Düzeltme:** `purge_disliked`: istenmeyen tüm şarkılar kuyruktan çıkarılıyor, çalan da istenmiyorsa ondan sonraki ilk uygun şarkıya geçiliyor (yoksa duraklatılıp bildiriliyor); bildirimde çıkarılan sayı.
- **Doğrulama:** Test kopyasında Haifisch çalarken kuyruktaki "Mein Land"de "Sanatçıyı önerme" → kuyruk 26→20, Chop Suey!'e geçildi.
- **Not:** Sanatçı alanı yükleyici olan şarkılar (ör. "Rammstein - Pussy", sanatçı "Eliass Kevrelis") eşleşmiyor.
- **Durum:** DÜZELTİLDİ (27 Kas).

### F8. Arapça (sağdan sola) sözler ters görünüyor (27 Kas 2026)
- **Bildiren:** yabancı bir kullanıcı (Instagram).
- **Neden (kod okuması, denenmedi):** Çalan satır `lyric-rows` içinde kelime kelime `HorizontalLayout` ile soldan sağa diziliyor (`ui/app.slint` ~2889, `KaraokeWord`). Sağdan sola dillerde kelime sırası bu yüzden ters çıkıyor; kelime dolumu da soldan sağa. Çalmayan satırlar düz `Text`; harf şekillendirmesi muhtemelen doğru ama hizası sola.
- **Yön:** Rust tarafında satırın yönünü belirle (Arapça, İbranice, Farsça, Urduca harf aralıkları). RTL satırda her `row`'daki kelimeleri ters sırala, hizayı sağa al (ortalı kipte ortada kalsın), `KaraokeWord` dolumunu sağdan sola yap. Satır kırma da sağdan başlamalı. Çeviri satırı kendi yönüne göre hizalansın. Test için Arapça senkronlu sözü olan bir şarkı (ör. Amr Diab).
- **Düzeltme:** `lyrics::dir` (ilk güçlü harf; harfsiz satır şarkının genel yönünü alır). Sağdan sola satırda kelimeler her görsel satırda ters dizilir, satır sağa yaslanır; `KaraokeWord` dolumu ve yumuşak kenarı sağdan başlar; `LyricWord.idx` dolum sırasını taşır. Çeviri satırı kendi yönüne göre, müzik arası noktaları da şarkının yönüne göre.
- **Doğrulama:** Test kopyasında Amr Diab "Tamally Maak" (YouTube sözü): satırlar sağda, çalan satırda kelime sırası doğru, dolum sağdan sola; Latin harfli sözlerde değişiklik yok. Şarkı başı müzik arası noktaları canlı denenmedi.
- **Durum:** DÜZELTİLDİ (27 Kas).


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
- **Durum:** DÜZELTİLDİ (27 Kas): ret iletisi okunuyor; tür yalnız "type" hatasında ve türsüz gönderim kabul edilirse bırakılıyor, yeniden bağlanınca sıfırlanıyor; diğer retlerde türü koruyan sade sürüm (kapak/zaman yok) gönderiliyor.

### Y2. Hale efektini diğer katmanlarla birleştirme (26 Kas 2026)
- **Görünüm:** Hale (`bg_style 4`) ışığı yalnız içerik alanının içinde kalıyor; sol barın kenarında (ve ana sayfa kartlarının arkasında) keskin bir çizgiyle kesiliyor, kapaktan taşan ışık sol bara ve diğer katmanlara geçmiyor. Ekran görüntüsü: `notes/fix/hale-katman.jpg` (CANTA LALA, ana sayfa).
- **İlgili kod:** `ui/parts.slint` `HaloGlow` (~satır 1122); `ui/app.slint` içinde üç ayrı yerde çiziliyor (~1106, ~2684, ~3057), her biri kendi kabının içinde kırpılıyor.
- **Düşünülecek:** Işığı tek bir katman olarak pencere zemininde (sol bar + içerik + alt çubuk) çizip barların yarı saydam zeminle üstüne binmesi; ya da sol barın ve alt çubuğun da kendi Hale yansımasını alması. Performans (ek bulanık katman) ve okunabilirlik (sol bar yazıları) birlikte değerlendirilmeli.
- **Durum:** Fikir aşamasında, karar verilmedi.

### Y3. Hale yayılma alanı sürgüsü (26 Kas 2026)
- **İstek:** Görünüm > Arka plan'da Hale seçiliyken renklerin ne kadar geniş yayılacağını ayarlayan bir sürgü.
- **İlgili kod:** `HaloGlow` zaten `spread` özelliğine sahip (`ui/parts.slint`, varsayılan 2.6 = kapağın 2.6 katı); şu an sabit. Ayara (`settings.json`, örn. `halo_spread`) bağlanıp üç çizim yerine (`ui/app.slint` ~1106, ~2684, ~3057) aktarılması yeterli olabilir.
- **Düşünülecek:** Aralık (örn. 1.5–5×), büyük değerde ışığın içerik kenarında kesilmesi (bkz. Y2, birlikte ele alınabilir), sürgü sürüklenirken canlı önizleme.
- **Yapılan (27 Kas):** Görünüm → Arka plan ışığı → Hale seçiliyken "Yayılım" sürgüsü (%60–180, `halo_spread` çarpanı; ana sayfa 3.2, sözler/önizleme 2.6 temel), sürüklerken canlı, bırakınca kaydedilir.
- **Durum:** YAPILDI (0.4.2).

### Y4. Hale renk seçicisinin geliştirilmesi (26 Kas 2026)
- **Görünüm:** Belirgin mor/beyaz kapakta (MONTAGEM ENTROPIA, duraklatılmış) Hale neredeyse hiç görünmüyor; vurgu rengi mor seçilmiş ama kapağın arkasında ışık yok denecek kadar sönük. Ekran görüntüsü: `notes/fix/hale-renk.png`. Karşılaştırma: turuncu/kırmızı kapakta (`notes/fix/hale-katman.jpg`) güçlü görünüyor.
- **İlgili kod:** `src/art.rs` `blur2` üretimi (~satır 64-88): bulanık kapağın her pikseli ters çevrilip HSL'de doygunluk ×1.15 (≤0.9), açıklık 0.42+l·0.4 bandına alınıyor. Renk kapağın ortalamasından geliyor, ayrı bir renk seçimi yok.
- **Analiz (Tahmin, doğrulanmadı):** Kapak çoğunlukla siyah + ince beyaz/mor ayrıntıdan oluşunca bulanıklık bunları gri-koyu mora ortalıyor; doygunluk düşük kaldığı için ışık zeminle kayboluyor. Duraklatılmışken `level`/`pulse` düşüyor olabilir, ayrıca kontrol edilmeli.
- **Geliştirme yönü:** Işığı ortalama yerine kapak paletinden (`palette`, accent) baskın + canlı renklerle kurmak; düşük doygunlukta vurgu rengine yaslanmak; isteğe bağlı elle renk seçimi (Otomatik / Vurgu / Özel). Y3 (yayılma sürgüsü) ile aynı ayar grubunda olabilir.
- **Yapılan (27 Kas):** `art.rs` `halo_vivid`/`halo_tint`: doygunluğu 0.35 altındaki pikseller kapak paletinin (ya da vurgunun) en canlı tonuna doğru boyanır; tamamen gri kapakta beyazımsı kalır. "Hale rengi": Kapaktan / Vurgu rengi (tek renkli radyal ışık). Ayrıca hata: kapak listede çok önce yüklendiyse bulanık sürümler LRU'dan (24) düşüyor, çalınca hale hiç çizilmiyordu → `set_now_ui` eksikse kapağı yeniden işletir.
- **Doğrulama:** Test kopyasında yeşil kapak (Padme) önce gri, şimdi yeşil hale; "Sattın Kendini" hale yoktu → var; vurgu rengi pembe, yayılım %160/%90 kaydı.
- **Durum:** YAPILDI (0.4.2). Elle renk seçici (Özel) yapılmadı; vurgu rengi paleti yeterli görüldü.

### Y5. Çoklu dil: İngilizce + isteğe bağlı diller (26 Kas 2026)
- **Kaynak:** GitHub issue #1 (nidego, Brezilya): arayüz yalnız Türkçe, İngilizce/Portekizce yok. 26 Kas'ta yanıtlandı: İngilizce "çok yakında", pt-BR listede, diğer diller istek üzerine; İngilizce çıkınca issue'ya haber verilecek.
- **Yön:** Slint `@tr()` + gettext (.po) ile arayüz metinlerini çeviri dosyalarına taşımak; Rust tarafındaki metinler (bildirimler, Yenilikler notları `notes/vX.Y.Z.md`, hata iletileri) de dahil. Ayarlar'a dil seçimi, varsayılan sistem dili.
- **Ek talep (27 Kas):** Instagram'dan yabancı bir kullanıcı da arayüzün yalnız Türkçe olduğundan şikâyet etti; ikinci bildirim.
- **Yapılan (27 Kas):** Slint `@tr` + gömülü `.po` (`lang/en/LC_MESSAGES/vynz.po`, bağlamsız; aynı dosyayı Rust `src/i18n.rs` `tr!`/`trn!` ile okur). ~490 metin. Ayarlar → Görünüm → "Dil · Language" (Sistem/Türkçe/English), canlı geçiş. Sistem dili Türkçe değilse İngilizce. YTM içerik dili/bölgesi arayüz diline bağlı (Türkçe: tr/TR, İngilizce: en + sistem bölgesi; `VYNZ_HL`/`VYNZ_GL` → resolver.py). Sürüm notu `notes/en/vX.Y.Z.md` (isteğe bağlı) ve release gövdesinde `<!-- en -->` ayracı; güncelleyici dile uyanı gösterir. Güncelleme betiği `lang/update.py`.
- **pt-BR (27 Kas):** `LANGS` listesiyle genelleştirildi; `lang/pt_BR`, Portekizce sistemde kendiliğinden; notlar `notes/pt_BR/` (yoksa İngilizce); resolver.py etiketleri. 0.4.1.
- **Kalan:** Tür listesi (Zevkler) Türkiye odaklı; İngilizce kullanıcıya sıra/öneri uyarlanabilir.
- **Durum:** İngilizce YAPILDI (0.4.0).

### Y6. Windows kod imzası (SignPath Foundation) (27 Kas 2026)
- **Hazır:** `CODE_SIGNING_POLICY.md`, README'de "Code signing policy" + SmartScreen notu, CI'da etikete bağlı iki imza adımı (vynz.exe, sonra setup.exe; `SIGNPATH_ORGANIZATION_ID` yoksa atlanır), `packaging/windows/signpath/` artifact yapılandırmaları, Inno'da sürüm meta verisi. Form cevapları depo dışında: `~/Projeler/vynz-arastirma/signpath/BASVURU.md`.
- **Kullanıcıda:** GitHub 2FA kontrolü, formu gönderme.
- **Riskler:** YouTube imza çözümü/PO token "güvenlik önlemi aşma" sayılabilir; ilk açılışta imzasız araç indirme (uv, Python, yt-dlp, deno).
- **Durum:** Başvuru gönderildi (27 Kas 2026, ad LURID Imura); yanıt bekleniyor. Zayıf nokta: itibar (≈50 indirme, 3 yıldız).

### F6. Yaş doğrulaması gereken şarkılar çalmıyor (27 Kas 2026)
- **Doğrulandı:** Test videosu `qkO6iBwcoe4` (Rammstein - Pussy, resmî klip). Yerli çözücü `ytfast` → `LOGIN_REQUIRED: Sign in to confirm your age`; yt-dlp (çerezsiz) → `Sign in to confirm your age`. Web yedeği de aynı oturumsuz istemciyle gidiyor. Çerezsiz hiçbir yol bu içeriği açamıyor.
- **Kısa vadeli düzeltme:** Hata yutulmasın: şarkı "Yaş doğrulaması gerekiyor" bildirimiyle atlansın (bkz. Y7 ile kalıcı çözüm). YTM'de aynı şarkının yaş kısıtsız Topic/ses sürümü varsa ona düşmek denenebilir.
- **Durum:** Kısa vadeli DÜZELTİLDİ (27 Kas): yerli çözücü yaş hatasında yedek yolları beklemiyor, "Yaş doğrulaması gerekiyor, atlandı" bildirimi + sonrakine geçiş (kuyruk radyoyla doluyorsa dolunca). Kalıcı çözüm Y7.

---

### Y7. İsteğe bağlı YouTube hesabı bağlama (yaş kısıtlı içerik için) (27 Kas 2026)
- **Fikir (kullanıcı):** Kullanıcı isterse YouTube hesabı bağlanır; yalnız yaş doğrulaması gereken şarkılarda hesap oturumuyla çözülür.
- **Yol:** Tarayıcıdan çerez içe aktarma (yt-dlp `cookies-from-browser` Firefox tabanlı Zen'i de okur) ya da Vynz içinde giriş penceresi; yerli çözücüde SAPISIDHASH ile oturumlu istek. Hesap yalnız LOGIN_REQUIRED/yaş hatasında kullanılır, normal çalmada değil.
- **Riskler:** Hesabın 18+ ve gerekirse kimlikle doğrulanmış olması şart; otomasyon nedeniyle hesap işaretlenme riski (ikincil hesap önerilir); çerezler yerelde, dosya izinleri kısıtlı saklanmalı; gizlilik politikasına (CODE_SIGNING_POLICY.md) eklenmeli.
- **Durum:** Fikir, onay bekliyor.

### Y8. Spotify girişiyle çalma (yalnız soru, ileride değerlendirilecek) (27 Kas 2026)
- librespot ile mümkün ama YALNIZ Premium hesapta; ücretsiz hesapta çalmaz. Spotify kullanım koşullarına aykırı, hesap riski ve Spotify'ın protokol değişikliklerinde kırılma riski var. Ücretsiz kullanıcılar için mevcut yol (meta veri alıp YTM'de eşleştirme) kalır.
- **Durum:** Ertelendi.
