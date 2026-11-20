//! Windows : l'icône de l'application et ses informations de version dans l'exécutable.

fn main() {
    println!("cargo:rerun-if-changed=../../build/icon.ico");
    #[cfg(windows)]
    {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("../../build/icon.ico");
        res.set("ProductName", "control");
        res.set("FileDescription", "control — lecteur musique et vidéo");
        res.set("LegalCopyright", "Copyright © 2026 nildontsleep");
        if let Err(err) = res.compile() {
            println!("cargo:warning=icône Windows non intégrée : {err}");
        }
    }
}
