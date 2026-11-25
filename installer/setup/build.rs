//! Embeds the resources of the setup and the app it installs.
//!
//! The app comes from `SSG_SETUP_PAYLOAD`, or from the workspace's release
//! build. Without it the setup still builds (for `cargo test` and clippy)
//! and refuses to install.

#[path = "../../build/resources.rs"]
mod resources;

use std::env;
use std::path::PathBuf;

fn main() {
    let here = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let root = here.join("..").join("..");
    resources::embed(
        &root,
        "Simple Screen Grabber Setup",
        "simple-screen-grabber-setup",
    );

    println!("cargo:rerun-if-env-changed=SSG_SETUP_PAYLOAD");
    let payload = env::var_os("SSG_SETUP_PAYLOAD")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("target")
                .join("release")
                .join("simple-screen-grabber.exe")
        });
    println!("cargo:rerun-if-changed={}", payload.display());
    let data = std::fs::read(&payload).unwrap_or_default();
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    std::fs::write(out.join("payload.bin"), data).expect("write payload.bin");
}
