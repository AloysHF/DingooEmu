use crate::a330::memory::LEGACY_FRAMEBUFFER_ADDRESS;
use crate::a330::runtime::Runtime;
use crate::package::PackageImage;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn doom_package() -> Option<(PackageImage, PathBuf)> {
    let path = std::env::var_os("DINGOOEMU_DOOM_CC")
        .map(PathBuf::from)
        .or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join("tmp/DOOM-A330.cc");
            path.exists().then_some(path)
        })?;
    let package = PackageImage::from_path(&path).ok()?;
    Some((package, path))
}

fn word_histogram(words: &[u32]) -> Vec<(u32, usize)> {
    let mut map = BTreeMap::new();
    for word in words {
        *map.entry(*word).or_insert(0usize) += 1;
    }
    map.into_iter().take(8).collect()
}

#[test]
fn doom_a330_runtime_snapshot() {
    let Some((package, path)) = doom_package() else {
        eprintln!("skip: DOOM-A330.cc not found");
        return;
    };
    let mut runtime = Runtime::from_package(package, path).expect("runtime");
    runtime.start();
    for frame in 1..=180 {
        if let Err(error) = runtime.tick() {
            print!("{}", runtime.debug_snapshot());
            panic!("tick {frame} failed: {error}");
        }
        if frame == 90 || frame == 180 {
            let mut legacy = Vec::new();
            for index in 0..256usize {
                let address = LEGACY_FRAMEBUFFER_ADDRESS + (index as u32) * 4;
                legacy.push(runtime.memory.read32(address).unwrap_or(0));
            }
            let mut nonzero = 0usize;
            for pixel in runtime.video.framebuffer().as_chunks::<2>().0 {
                if *pixel != [0, 0] {
                    nonzero += 1;
                }
            }
            println!(
                "frame={frame} host_frames={} host_nonzero={nonzero} legacy_hist={:?}\n{}",
                runtime.video.frame_count(),
                word_histogram(&legacy),
                runtime.debug_snapshot()
            );
        }
    }
}
