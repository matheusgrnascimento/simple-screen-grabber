//! Embeds the manifest, the icon and the version in the executable (see
//! `build/resources.rs`).

#[path = "build/resources.rs"]
mod resources;

fn main() {
    let root = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    resources::embed(&root, "Simple Screen Grabber", "simple-screen-grabber");
}
