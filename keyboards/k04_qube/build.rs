use const_gen::*;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::{env, fs};
use xz2::read::XzEncoder;

fn main() {
    const FIRMWARE_VERSION: &str = "0.1.3";
    const FIRMWARE_VERSION_BCD: &str = "0x0103";

    let vial_path = configured_path("VIAL_JSON_PATH", "vial.json");
    let keyboard_path = configured_path("KEYBOARD_TOML_PATH", "keyboard.toml");

    println!("cargo:rerun-if-env-changed=VIAL_JSON_PATH");
    println!("cargo:rerun-if-env-changed=KEYBOARD_TOML_PATH");
    println!("cargo:rerun-if-changed={}", vial_path.display());
    println!("cargo:rerun-if-changed={}", keyboard_path.display());
    println!("cargo:rerun-if-changed=memory_halves.x");
    println!("cargo:rerun-if-changed=memory_qube.x");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION={FIRMWARE_VERSION}");
    println!("cargo:rustc-env=RMK_FIRMWARE_VERSION_BCD={FIRMWARE_VERSION_BCD}");

    if env::var_os("CARGO_FEATURE_QUBE").is_some() {
        println!("cargo:rustc-env=RMK_VIAL_DEVICE_SETTINGS_FN=crate::layer_names::vial_device_settings");
    }

    generate_vial_config(&vial_path);

    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let memory = if env::var_os("CARGO_FEATURE_QUBE").is_some() {
        include_bytes!("memory_qube.x").as_slice()
    } else {
        include_bytes!("memory_halves.x").as_slice()
    };
    File::create(out.join("memory.x")).unwrap().write_all(memory).unwrap();
    println!("cargo:rustc-link-search={}", out.display());

    println!("cargo:rustc-link-arg=--nmagic");
    println!("cargo:rustc-link-arg=-Tlink.x");
    println!("cargo:rustc-link-arg=-Tdefmt.x");
    println!("cargo:rustc-linker=flip-link");
}

fn configured_path(variable: &str, default: &str) -> PathBuf {
    env::var_os(variable)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(default))
}

fn generate_vial_config(vial_path: &Path) {
    let out_file = Path::new(&env::var_os("OUT_DIR").unwrap()).join("config_generated.rs");

    let mut content = String::new();
    match File::open(vial_path) {
        Ok(mut file) => {
            file.read_to_string(&mut content)
                .unwrap_or_else(|e| panic!("Cannot read {}: {e}", vial_path.display()));
        }
        Err(e) => panic!("Cannot find {}: {e}", vial_path.display()),
    };

    let parsed = json::parse(&content).unwrap_or_else(|e| panic!("Cannot parse {}: {e}", vial_path.display()));
    let product_id = parsed["productId"]
        .as_str()
        .and_then(|value| value.strip_prefix("0x").or_else(|| value.strip_prefix("0X")))
        .and_then(|value| u16::from_str_radix(value, 16).ok())
        .unwrap_or_else(|| panic!("{} productId must be a hexadecimal string", vial_path.display()));
    let mut vial_cfg = json::stringify(parsed);
    if !vial_cfg.contains("\"entropy\"") {
        vial_cfg.insert_str(
            1,
            "\"entropy\":{\"liveFeatures\":[\"time\",\"media\"],\"batteryHalves\":true},",
        );
    }
    let mut keyboard_def_compressed: Vec<u8> = Vec::new();
    XzEncoder::new(vial_cfg.as_bytes(), 6)
        .read_to_end(&mut keyboard_def_compressed)
        .unwrap();

    let keyboard_id: Vec<u8> = match product_id {
        // Preserve the established K:04 and Mini identities; Micro gets a
        // dedicated ID so Vial cannot load the Mini layout from its cache.
        0x0071 => vec![0x80, 0x04, 0x28, 0xAB, 0x69, 0x3E, 0x19, 0x60],
        0x0072 => vec![0x80, 0x04, 0x2D, 0x7A, 0x91, 0x44, 0x3B, 0x21],
        0x0073 => vec![0x80, 0x04, b'Q', b'0', b'4', b'M', b'I', b'C'],
        _ => panic!("Unsupported K:04 Qube productId: 0x{product_id:04X}"),
    };
    let const_declarations = [
        const_declaration!(pub VIAL_KEYBOARD_DEF = keyboard_def_compressed),
        const_declaration!(pub VIAL_KEYBOARD_ID = keyboard_id),
    ]
    .map(|s| "#[allow(clippy::redundant_static_lifetimes)]\n".to_owned() + s.as_str())
    .join("\n");
    fs::write(out_file, const_declarations).unwrap();
}
