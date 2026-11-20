use std::{env, path::PathBuf, process::Command};

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
        let protocol = "linux/focus-grab.xml";
        println!("cargo:rerun-if-changed={protocol}");
        println!("cargo:rerun-if-changed=linux/panel.c");
        for (mode, file) in [("client-header", "focus-grab.h"), ("private-code", "focus-grab.c")] {
            let status = Command::new("wayland-scanner").arg(mode).arg(protocol)
                .arg(out.join(file)).status().expect("Instale wayland-scanner para compilar no Linux");
            assert!(status.success(), "Falha ao gerar a interface Wayland");
        }
        let mut build = cc::Build::new();
        for library in ["gtk+-3.0", "wayland-client"] {
            build.includes(pkg_config::probe_library(library).unwrap().include_paths);
        }
        build.include(&out).file(out.join("focus-grab.c")).file("linux/panel.c")
            .flag_if_supported("-Wno-unused-parameter").compile("ayla_panel");
    }
    tauri_build::build()
}
