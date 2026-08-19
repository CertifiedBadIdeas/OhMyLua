use std::process::Command;

fn main() {
    let output = Command::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .expect("failed to run pinned rustc to determine its sysroot");

    assert!(
        output.status.success(),
        "pinned rustc failed while determining its sysroot"
    );

    let sysroot = String::from_utf8(output.stdout)
        .expect("rustc sysroot is not valid UTF-8")
        .trim()
        .to_owned();

    assert!(!sysroot.is_empty(), "rustc returned an empty sysroot");

    println!("cargo:rustc-env=OMLUA_RUSTC_SYSROOT={sysroot}");
}
