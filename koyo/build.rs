fn main() {
    #[cfg(windows)]
    {
        let mut resource = winres::WindowsResource::new();
        resource.set_icon("assets/koyo.ico");
        resource.set("FileDescription", "Koyo");
        resource.set("ProductName", "Koyo");
        resource.compile().expect("embed the Koyo Windows icon");
    }
}
