fn main() {
    // Embed the application icon into the Windows executable so the
    // taskbar/Explorer show the blackhole icon.
    if std::env::var("CARGO_CFG_WINDOWS").is_ok() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/blackhole.ico");
        res.compile().expect("compile windows resources");
    }
}
