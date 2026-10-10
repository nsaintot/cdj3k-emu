//! Scratch folders and mod archives for the mods tests.

use std::path::{Path, PathBuf};

use super::{LOADER, MANIFEST};

pub(super) fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("cdj3k-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(super) fn mod_folder(parent: &Path, folder: &str, name: &str, version: &str) -> PathBuf {
    let dir = parent.join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join(MANIFEST),
        format!("[mod]\nname = \"{name}\"\nversion = \"{version}\"\n"),
    )
    .unwrap();
    std::fs::write(dir.join(LOADER), "echo hi\n").unwrap();
    dir
}

pub(super) fn tgz(src: &Path, dest: &Path, wrapped: bool) {
    let gz = flate2::write::GzEncoder::new(
        std::fs::File::create(dest).unwrap(),
        flate2::Compression::fast(),
    );
    let mut tar = tar::Builder::new(gz);
    if wrapped {
        tar.append_dir_all("folder", src).unwrap();
    } else {
        for f in [MANIFEST, LOADER] {
            tar.append_path_with_name(src.join(f), f).unwrap();
        }
    }
    tar.into_inner().unwrap().finish().unwrap();
}

pub(super) fn tar_gz(
    dest: &Path,
    fill: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>),
) {
    let gz = flate2::write::GzEncoder::new(
        std::fs::File::create(dest).unwrap(),
        flate2::Compression::fast(),
    );
    let mut tar = tar::Builder::new(gz);
    fill(&mut tar);
    tar.into_inner().unwrap().finish().unwrap();
}

pub(super) fn add_leftovers(dir: &Path) -> usize {
    std::fs::read_dir(dir).map_or(0, |entries| {
        entries
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".add-")
            })
            .count()
    })
}
