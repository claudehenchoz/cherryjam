fn main() {
    // Embed the application icon in the Windows executable.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        println!("cargo:rerun-if-changed=icons/windows/cherryjam.ico");
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/windows/cherryjam.ico");
        if let Err(e) = res.compile() {
            println!("cargo:warning=could not embed icon: {e}");
        }
    }
}
