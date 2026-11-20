fn main() {
    slint_build::compile("ui/app.slint").expect("slint derlenemedi");
    #[cfg(target_os = "windows")]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("design/logo/vynz.ico");
        res.set("ProductName", "Vynz");
        res.set("FileDescription", "Vynz Müzik Çalar");
        if let Err(e) = res.compile() {
            println!("cargo:warning=winresource hatası: {e}");
        }
    }
}
