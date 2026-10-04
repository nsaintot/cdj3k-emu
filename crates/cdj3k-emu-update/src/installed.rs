//! How this build was installed. Each release package carries a one-line
//! `package-kind` file in the payload directory; a developer's build has none.

use crate::Kind;

pub fn installed_kind() -> Option<Kind> {
    let path = cdj3k_emu_platform::bundled::resources().join("package-kind");
    Kind::parse(&std::fs::read_to_string(path).ok()?)
}
