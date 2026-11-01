use std::{env, path::PathBuf, process::Command};
fn main() {
    println!("cargo:rerun-if-changed=assets/app.ico");
    println!("cargo:rerun-if-changed=assets/app.rc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let root = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let gnu = env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");
    let output = out.join(if gnu { "app-icon.o" } else { "app-icon.res" });
    let mut command = Command::new(if gnu { "windres.exe" } else { "rc.exe" });
    command.current_dir(&root);
    if gnu {
        command
            .arg("-I")
            .arg(&root)
            .args(["-i", "assets/app.rc", "-o"])
            .arg(&output);
    } else {
        command
            .args(["/nologo", "/fo"])
            .arg(&output)
            .arg("assets/app.rc");
    }
    let status = command
        .status()
        .expect("Install windres (MinGW) or rc.exe (Windows SDK) to embed the icon");
    assert!(status.success(), "Icon resource compiler failed");
    println!("cargo:rustc-link-arg-bin=clipbridge={}", output.display());
}
