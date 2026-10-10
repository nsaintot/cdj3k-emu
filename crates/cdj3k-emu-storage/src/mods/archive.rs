//! Unpacking a mod archive: a `.tgz`, `.tar.gz` or `.tar` holding files and
//! folders only, within a size and an entry limit.

use std::io;
use std::path::Path;

/// The maximum total size an archive may unpack to.
const UNPACKED_LIMIT: u64 = 1 << 30;
/// The most files and folders an archive may hold.
const ENTRY_LIMIT: usize = 10_000;

/// Unpack `archive` into `into`. Only files and folders are accepted, up to
/// [`ENTRY_LIMIT`] entries and [`UNPACKED_LIMIT`] bytes in total.
pub(super) fn unpack(archive: &Path, into: &Path) -> io::Result<()> {
    std::fs::create_dir_all(into)?;
    let file = std::fs::File::open(archive)?;
    let read: Box<dyn io::Read> = if archive.to_string_lossy().ends_with(".tar") {
        Box::new(file)
    } else {
        Box::new(flate2::read::GzDecoder::new(file))
    };
    let fail = |what: String| io::Error::other(format!("{}: {what}", archive.display()));
    // Tar headers count towards the limit too: a long-name header's body is
    // read into memory before its entry is seen.
    let read = Capped {
        inner: read,
        left: UNPACKED_LIMIT,
    };
    let mut tar = tar::Archive::new(read);
    let mut total = 0u64;
    for (count, entry) in tar.entries().map_err(|e| fail(e.to_string()))?.enumerate() {
        if count >= ENTRY_LIMIT {
            return Err(fail(format!(
                "holds more than {ENTRY_LIMIT} files and folders"
            )));
        }
        let mut entry = entry.map_err(|e| fail(e.to_string()))?;
        let path = entry
            .path()
            .map_or_else(|_| "?".into(), |p| p.display().to_string());
        let kind = entry.header().entry_type();
        // Archives from `git archive` and GitHub start with a pax header that
        // names the commit.
        if kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
            continue;
        }
        if !(kind.is_file() || kind.is_dir()) {
            return Err(fail(format!(
                "{path} is not a file or a folder; a mod archive may contain only \
                 files and folders"
            )));
        }
        // Unpacking applies the entry's mode. An entry its owner cannot read
        // could be neither booted nor removed.
        let needs = if kind.is_dir() { 0o700 } else { 0o600 };
        if entry.header().mode().is_ok_and(|m| m & needs != needs) {
            return Err(fail(format!("{path}: its owner cannot read it")));
        }
        total += entry.size();
        if total > UNPACKED_LIMIT {
            return Err(fail(format!(
                "unpacks to more than {} MiB",
                UNPACKED_LIMIT >> 20
            )));
        }
        if !entry
            .unpack_in(into)
            .map_err(|e| fail(format!("{path}: {e}")))?
        {
            return Err(fail(format!("{path} points outside the archive")));
        }
    }
    Ok(())
}

/// A reader that fails once `left` bytes have been read.
struct Capped<R> {
    inner: R,
    left: u64,
}

impl<R: io::Read> io::Read for Capped<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.left == 0 {
            return Err(io::Error::other(format!(
                "unpacks to more than {} MiB",
                UNPACKED_LIMIT >> 20
            )));
        }
        let max = buf
            .len()
            .min(usize::try_from(self.left).unwrap_or(usize::MAX));
        let n = self.inner.read(&mut buf[..max])?;
        self.left -= n as u64;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mods::test_support::*;
    use crate::mods::{SlotMods, LOADER, MANIFEST};

    #[test]
    fn an_archive_with_a_link_is_refused() {
        let root = scratch("link");
        let src = mod_folder(&root, "src", "linked", "1");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(root.join("linked.tgz")).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        tar.append_path_with_name(src.join(MANIFEST), MANIFEST)
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Symlink);
        header.set_size(0);
        tar.append_link(&mut header, "secret", "/etc/passwd")
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let mods = SlotMods::load_in(root.join("mods"));
        let err = mods.prepare(&root.join("linked.tgz"), None).unwrap_err();
        assert!(err.to_string().contains("secret"), "{err}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_git_archive_installs() {
        let root = scratch("git-archive");
        let src = mod_folder(&root, "src", "from-git", "1");
        let archive = root.join("from-git.tar.gz");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(&archive).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        let comment = b"52 comment=0123456789abcdef0123456789abcdef01234567\n";
        let mut header = tar::Header::new_ustar();
        header.set_path("pax_global_header").unwrap();
        header.set_entry_type(tar::EntryType::XGlobalHeader);
        header.set_size(comment.len() as u64);
        header.set_cksum();
        tar.append(&header, &comment[..]).unwrap();
        for f in [MANIFEST, LOADER] {
            tar.append_path_with_name(src.join(f), format!("from-git/{f}"))
                .unwrap();
        }
        tar.into_inner().unwrap().finish().unwrap();
        let mut mods = SlotMods::load_in(root.join("mods"));
        assert_eq!(mods.add(&archive, None).unwrap(), "from-git");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn an_archive_entry_its_owner_cannot_read_is_refused() {
        let root = scratch("mode");
        let src = mod_folder(&root, "src", "locked", "1");
        let gz = flate2::write::GzEncoder::new(
            std::fs::File::create(root.join("locked.tgz")).unwrap(),
            flate2::Compression::fast(),
        );
        let mut tar = tar::Builder::new(gz);
        tar.append_path_with_name(src.join(MANIFEST), MANIFEST)
            .unwrap();
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Directory);
        header.set_mode(0o000);
        header.set_size(0);
        tar.append_data(&mut header, "lib", std::io::empty())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let mods = SlotMods::load_in(root.join("mods"));
        let err = mods.prepare(&root.join("locked.tgz"), None).unwrap_err();
        assert!(err.to_string().contains("lib"), "{err}");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_plain_tar_installs() {
        let root = scratch("plain-tar");
        let src = mod_folder(&root, "src", "plain", "1");
        let archive = root.join("plain.tar");
        let mut tar = tar::Builder::new(std::fs::File::create(&archive).unwrap());
        tar.append_dir_all("plain", &src).unwrap();
        tar.into_inner().unwrap();
        let mut mods = SlotMods::load_in(root.join("mods"));
        assert_eq!(mods.add(&archive, None).unwrap(), "plain");
        assert!(root.join("mods/installed/plain").join(LOADER).is_file());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn unpack_refusals_have_their_messages() {
        let root = scratch("unpack-msgs");
        let src = mod_folder(&root, "src", "x", "1");
        let mods = SlotMods::load_in(root.join("mods"));
        let refused = |name: &str, fill: &dyn Fn(&mut tar::Builder<_>)| {
            let archive = root.join(name);
            tar_gz(&archive, |tar| {
                tar.append_path_with_name(src.join(MANIFEST), MANIFEST)
                    .unwrap();
                fill(tar);
            });
            let err = mods.prepare(&archive, None).unwrap_err().to_string();
            (archive.display().to_string(), err)
        };

        let (path, err) = refused("link.tgz", &|tar| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Symlink);
            header.set_size(0);
            tar.append_link(&mut header, "secret", "/etc/passwd")
                .unwrap();
        });
        assert_eq!(
            err,
            format!(
                "{path}: secret is not a file or a folder; a mod archive may contain only \
                 files and folders"
            )
        );

        let (path, err) = refused("mode.tgz", &|tar| {
            let mut header = tar::Header::new_gnu();
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o200);
            header.set_size(0);
            tar.append_data(&mut header, "locked.so", std::io::empty())
                .unwrap();
        });
        assert_eq!(err, format!("{path}: locked.so: its owner cannot read it"));

        let (path, err) = refused("outside.tgz", &|tar| {
            let mut header = tar::Header::new_old();
            header.as_old_mut().name[..9].copy_from_slice(b"../escape");
            header.set_entry_type(tar::EntryType::Regular);
            header.set_mode(0o644);
            header.set_size(0);
            header.set_cksum();
            tar.append(&header, std::io::empty()).unwrap();
        });
        assert_eq!(err, format!("{path}: ../escape points outside the archive"));

        let (path, err) = refused("many.tgz", &|tar| {
            for i in 0..ENTRY_LIMIT {
                let mut header = tar::Header::new_gnu();
                header.set_entry_type(tar::EntryType::Regular);
                header.set_mode(0o644);
                header.set_size(0);
                tar.append_data(&mut header, format!("f{i}"), std::io::empty())
                    .unwrap();
            }
        });
        assert_eq!(
            err,
            format!("{path}: holds more than 10000 files and folders")
        );

        assert_eq!(add_leftovers(&root.join("mods")), 0);
        std::fs::remove_dir_all(root).unwrap();
    }
}
