//! tuxread-cli.exe's Windows version resource (spec §9): the product name and version that
//! TuxRead.exe carries too, so both say which release they belong to.
fn main() {
    #[cfg(windows)]
    {
        let mut resource = tauri_winres::WindowsResource::new();
        resource
            .set("ProductName", "TuxRead")
            .set("FileDescription", "TuxRead command line")
            .set("LegalCopyright", "Copyright (c) 2026 the TuxRead authors")
            .set_icon("../../src-tauri/icons/icon.ico");
        if let Err(e) = resource.compile() {
            panic!("tuxread-cli's version resource: {e}");
        }
    }
}
