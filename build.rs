use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/xmouse.rc");
    println!("cargo:rerun-if-changed=assets/xmouse.ico");

    let target = env::var("TARGET").expect("TARGET is set by Cargo");
    let out_dir = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR is set by Cargo"));
    let resource_script = PathBuf::from("assets").join("xmouse.rc");

    if target.contains("windows-gnu") {
        let output = out_dir.join("xmouse-resource.o");
        run(
            Command::new("windres")
                .arg("-I")
                .arg("assets")
                .arg(&resource_script)
                .arg(&output),
            "windres",
        );
        println!("cargo:rustc-link-arg={}", output.display());
    } else if target.contains("windows-msvc") {
        let output = out_dir.join("xmouse-resource.res");
        run(
            Command::new("rc.exe")
                .arg("/nologo")
                .arg(format!("/fo{}", output.display()))
                .arg(&resource_script),
            "rc.exe",
        );
        println!("cargo:rustc-link-arg={}", output.display());
    }
}

fn run(command: &mut Command, tool: &str) {
    let status = command
        .status()
        .unwrap_or_else(|error| panic!("failed to launch {tool}: {error}"));
    assert!(status.success(), "{tool} failed with status {status}");
}
