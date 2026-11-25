//! Windows resources for the binaries, without extra tools or crates:
//!
//! - `app.manifest`, through the MSVC linker (`/MANIFEST:EMBED`);
//! - the app icon (`assets/icon.ico`) and the version information, in a
//!   `.res` file written here and passed to the linker.
//!
//! Shared by the build scripts of the app and of the setup.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

const RT_ICON: u16 = 3;
const RT_GROUP_ICON: u16 = 14;
const RT_VERSION: u16 = 16;
/// The app icon's resource id, used by the app to load it.
const ICON_ID: u16 = 1;
const LANG_EN_US: u16 = 0x0409;

/// Embeds `root/app.manifest`, `root/assets/icon.ico` and version
/// information named `description` (file `file_name`) into the binaries of
/// the package being built.
pub fn embed(root: &Path, description: &str, file_name: &str) {
    let out = PathBuf::from(env::var("OUT_DIR").expect("set by cargo"));
    let manifest = root.join("app.manifest");
    let icon = root.join("assets").join("icon.ico");
    println!("cargo:rerun-if-changed={}", manifest.display());
    println!("cargo:rerun-if-changed={}", icon.display());

    if env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc") {
        panic!("Simple Screen Grabber builds with the MSVC toolchain only");
    }

    let res = out.join("app.res");
    fs::write(&res, resources(&icon, description, file_name)).expect("write app.res");

    println!("cargo:rustc-link-arg-bins=/MANIFEST:EMBED");
    println!(
        "cargo:rustc-link-arg-bins=/MANIFESTINPUT:{}",
        manifest.display()
    );
    println!("cargo:rustc-link-arg-bins=/MANIFESTUAC:NO");
    println!("cargo:rustc-link-arg-bins={}", res.display());
}

/// Builds a `.res` file: the icon images, their group and the version.
fn resources(icon_path: &Path, description: &str, file_name: &str) -> Vec<u8> {
    let ico = fs::read(icon_path).expect("read assets/icon.ico (run: cargo run -p genicon)");
    let mut res = Vec::new();
    // A .res file starts with an empty entry.
    resource(&mut res, 0, 0, 0, &[]);

    let count = u16::from_le_bytes([ico[4], ico[5]]);
    let mut group = Vec::new();
    group.extend_from_slice(&0u16.to_le_bytes());
    group.extend_from_slice(&1u16.to_le_bytes());
    group.extend_from_slice(&count.to_le_bytes());
    for i in 0..count {
        let entry = &ico[6 + 16 * usize::from(i)..][..16];
        let size = u32::from_le_bytes(entry[8..12].try_into().expect("4 bytes")) as usize;
        let offset = u32::from_le_bytes(entry[12..16].try_into().expect("4 bytes")) as usize;
        let id = i + 1;
        resource(&mut res, RT_ICON, id, 0, &ico[offset..offset + size]);
        // GRPICONDIRENTRY: the ICONDIRENTRY fields, with the resource id in
        // place of the file offset.
        group.extend_from_slice(&entry[..12]);
        group.extend_from_slice(&id.to_le_bytes());
    }
    resource(&mut res, RT_GROUP_ICON, ICON_ID, 0, &group);
    resource(
        &mut res,
        RT_VERSION,
        1,
        LANG_EN_US,
        &version_info(description, file_name),
    );
    res
}

/// Appends one resource with an ordinal type and name.
fn resource(res: &mut Vec<u8>, kind: u16, name: u16, language: u16, data: &[u8]) {
    let header_size = 32u32;
    res.extend_from_slice(&(data.len() as u32).to_le_bytes());
    res.extend_from_slice(&header_size.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&kind.to_le_bytes());
    res.extend_from_slice(&[0xFF, 0xFF]);
    res.extend_from_slice(&name.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // DataVersion
    let flags: u16 = if kind == 0 { 0 } else { 0x1030 }; // MOVEABLE | PURE | DISCARDABLE
    res.extend_from_slice(&flags.to_le_bytes());
    res.extend_from_slice(&language.to_le_bytes());
    res.extend_from_slice(&0u32.to_le_bytes()); // Version
    res.extend_from_slice(&0u32.to_le_bytes()); // Characteristics
    res.extend_from_slice(data);
    pad(res);
}

fn pad(buf: &mut Vec<u8>) {
    while !buf.len().is_multiple_of(4) {
        buf.push(0);
    }
}

fn utf16z(s: &str) -> Vec<u8> {
    s.encode_utf16()
        .chain([0])
        .flat_map(u16::to_le_bytes)
        .collect()
}

/// One node of the version resource tree.
fn block(key: &str, value: &[u8], value_length: u16, text: bool, children: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = vec![0, 0];
    buf.extend_from_slice(&value_length.to_le_bytes());
    buf.extend_from_slice(&u16::from(text).to_le_bytes());
    buf.extend_from_slice(&utf16z(key));
    pad(&mut buf);
    buf.extend_from_slice(value);
    for child in children {
        pad(&mut buf);
        buf.extend_from_slice(child);
    }
    let length = buf.len() as u16;
    buf[..2].copy_from_slice(&length.to_le_bytes());
    buf
}

fn string(key: &str, value: &str) -> Vec<u8> {
    let chars = value.encode_utf16().count() as u16 + 1;
    block(key, &utf16z(value), chars, true, &[])
}

/// VS_VERSIONINFO with the package version and names.
fn version_info(description: &str, file_name: &str) -> Vec<u8> {
    let version = env::var("CARGO_PKG_VERSION").expect("set by cargo");
    let mut parts = version
        .split(['.', '-'])
        .map(|p| p.parse::<u16>().unwrap_or(0))
        .chain(std::iter::repeat(0));
    let (major, minor, patch) = (
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
        parts.next().unwrap_or(0),
    );
    let ms = (u32::from(major) << 16) | u32::from(minor);
    let ls = u32::from(patch) << 16;

    let mut fixed = Vec::new();
    for value in [
        0xFEEF_04BD, // signature
        0x0001_0000, // structure version
        ms,
        ls, // file version
        ms,
        ls,          // product version
        0x3F,        // flags mask
        0,           // flags
        0x0004_0004, // VOS_NT_WINDOWS32
        1,           // VFT_APP
        0,           // subtype
        0,
        0, // date
    ] {
        fixed.extend_from_slice(&u32::to_le_bytes(value));
    }

    let full = format!("{major}.{minor}.{patch}.0");
    let strings = [
        string("CompanyName", "Matheus Nascimento"),
        string("FileDescription", description),
        string("FileVersion", &full),
        string("InternalName", file_name),
        string(
            "LegalCopyright",
            "Copyright (c) 2026 Matheus Nascimento. MIT License.",
        ),
        string("OriginalFilename", &format!("{file_name}.exe")),
        string("ProductName", "Simple Screen Grabber"),
        string("ProductVersion", &version),
    ];
    let table = block("040904B0", &[], 0, true, &strings);
    let string_info = block("StringFileInfo", &[], 0, true, &[table]);
    let translation = (0x04B0u32 << 16 | u32::from(LANG_EN_US)).to_le_bytes();
    let var = block("Translation", &translation, 4, false, &[]);
    let var_info = block("VarFileInfo", &[], 0, true, &[var]);
    block(
        "VS_VERSION_INFO",
        &fixed,
        fixed.len() as u16,
        false,
        &[string_info, var_info],
    )
}
