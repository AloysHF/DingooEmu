use dingooemu_core::package::PackageImage;
use dingooemu_core::Emulator;
use std::path::{Path, PathBuf};

fn doom_path() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("DINGOOEMU_DOOM_CC").map(PathBuf::from) {
        return path.exists().then_some(path);
    }
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tmp/DOOM-A330.cc");
    path.exists().then_some(path)
}

#[test]
fn dump_doom_imports() {
    let Some(path) = doom_path() else {
        eprintln!("skip");
        return;
    };
    let package = PackageImage::from_path(&path).expect("package");
    println!("imports={}", package.imports.len());
    for (index, import) in package.imports.iter().enumerate() {
        println!("{index:03} {}", import.name);
    }
    let _ = Emulator::from_path(&path);
}
