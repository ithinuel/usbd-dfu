use std::env;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
fn main() -> Result<(), Box<(dyn std::error::Error + 'static)>> {
    let out = &PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let memory_file = if env::var("CARGO_FEATURE_NUCLEO_F401RE").is_ok() {
        "nucleo-f401re"
    } else if env::var("CARGO_FEATURE_DUET3D").is_ok() {
        "duet3d"
    } else {
        panic!("You must select a target feature.");
    };

    let mode = if env::var("CARGO_FEATURE_APPLICATION").is_ok() {
        "-application"
    } else if env::var("CARGO_FEATURE_BOOTLOADER").is_ok() {
        "-bootloader"
    } else {
        ""
    };

    let memory = std::fs::read(&format!("{}{}.x", memory_file, mode))?;
    File::create(out.join("memory.x"))
        .unwrap()
        .write_all(&memory)
        .unwrap();
    println!("cargo:rustc-link-search={}", out.display());
    println!("cargo:rerun-if-changed={}", memory_file);
    println!("cargo:rerun-if-changed=build.rs");
    Ok(())
}
