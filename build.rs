fn main() {
    // Kaynak dil Türkçe; çeviriler lang/<dil>/LC_MESSAGES/vynz.po. Bağlam yok:
    // aynı .po dosyasını Rust tarafındaki metinler de kullanır (src/i18n.rs).
    let cfg = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("lang")
        .with_default_translation_context(slint_build::DefaultTranslationContext::None);
    slint_build::compile_with_config("ui/app.slint", cfg).expect("slint derlenemedi");
    println!("cargo:rerun-if-changed=lang");
    // Çevrilmiş sürüm notları isteğe bağlı: notes/<dil>/vX.Y.Z.md yoksa boş gömülür
    // (uygulama İngilizceye, o da yoksa Türkçeye düşer).
    let ver = std::env::var("CARGO_PKG_VERSION").unwrap();
    let out_dir = std::env::var("OUT_DIR").unwrap();
    for lang in ["en", "pt_BR"] {
        let src = format!("notes/{lang}/v{ver}.md");
        println!("cargo:rerun-if-changed={src}");
        println!("cargo:rerun-if-changed=notes/{lang}");
        let text = std::fs::read_to_string(&src).unwrap_or_default();
        let out = std::path::Path::new(&out_dir).join(format!("notes_{lang}.md"));
        std::fs::write(out, text).expect("sürüm notu yazılamadı");
    }
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("design/logo/vynz.ico");
        res.set("ProductName", "Vynz");
        res.set("FileDescription", "Vynz Music Player");
        // İkon gömülemezse sessizce geçme: exe ikonsuz çıkıyordu (vynz.ico aslında PNG idi)
        res.compile().expect("winresource: ikon/kaynak gömülemedi");
    }
}
