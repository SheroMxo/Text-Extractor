use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=icon.ico");
    std::fs::write("resource.rc", "1 ICON \"icon.ico\"\n").expect("Failed to write resource.rc");

    let status = Command::new("windres")
        .args(["-i", "resource.rc", "-o", "resource.o"])
        .status()
        .expect("Failed to run windres");

    assert!(status.success(), "windres failed");
    println!("cargo:rustc-link-arg-bins=resource.o");
}
