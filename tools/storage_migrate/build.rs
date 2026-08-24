use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let memory = if env::var_os("CARGO_FEATURE_QUBE").is_some() {
        include_bytes!("memory_qube.x").as_slice()
    } else {
        include_bytes!("memory_halves.x").as_slice()
    };
    File::create(out.join("memory.x")).unwrap().write_all(memory).unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed=memory_halves.x");
    println!("cargo:rerun-if-changed=memory_qube.x");
}
