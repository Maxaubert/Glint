fn main() {
    // Exe icon and version info (Explorer, Start menu, Task Manager).
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../assets/glint.ico");
        res.set("FileDescription", "Glint");
        res.set("ProductName", "Glint");
        res.compile().expect("embed the Windows resources");
    }
}
