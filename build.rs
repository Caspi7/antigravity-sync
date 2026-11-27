fn main() {
    slint_build::compile("ui/app.slint").expect("Failed to compile ui/app.slint");

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        let icon_path = "assets/AntigravitySync.ico";
        if std::path::Path::new(icon_path).exists() {
            res.set_icon(icon_path);
        }
        res.set("ProductName", "Antigravity Sync");
        res.set("FileDescription", "Antigravity Sync");
        res.set("FileVersion", "2.1.7.0");
        res.set("ProductVersion", "2.1.7");
        if res.compile().is_ok()
            && std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
        {
            let resource = std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap())
                .join("resource.o");
            println!(
                "cargo:rustc-link-arg-bin=AntigravitySync={}",
                resource.display()
            );
        }
    }
}
