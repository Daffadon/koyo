fn main() {
    println!("cargo:rerun-if-changed=assets/koyo.ico");
    println!("cargo:rerun-if-changed=build.rs");

    #[cfg(windows)]
    {
        if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
            return;
        }

        let mut resource = winresource::WindowsResource::new();
        resource.set_icon("assets/koyo.ico");
        resource.set("FileDescription", "Koyo");
        resource.set("ProductName", "Koyo");
        resource.compile().expect("embed the Koyo Windows icon");
    }
}
