fn main() {
    slint_build::compile("ui/app.slint").expect("slint derlenemedi");
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
