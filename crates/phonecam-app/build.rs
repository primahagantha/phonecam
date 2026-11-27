use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=../../assets/icon.ico");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return,
    };
    let rc_script = manifest_dir.join("app.rc");
    let icon_path = manifest_dir.join("../../assets/icon.ico");
    if !rc_script.is_file() || !icon_path.is_file() {
        println!(
            "cargo:warning=Skipping Windows icon embedding: missing {} or {}",
            rc_script.display(),
            icon_path.display()
        );
        return;
    }

    let Some(rc_exe) = find_rc_exe() else {
        println!("cargo:warning=rc.exe not found; skipping embedded Windows executable icon");
        return;
    };

    let out_dir = match env::var("OUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return,
    };
    let out_res = out_dir.join("app.res");

    let status = Command::new(&rc_exe)
        .arg("/nologo")
        .arg("/fo")
        .arg(&out_res)
        .arg(&rc_script)
        .status();

    match status {
        Ok(exit) if exit.success() && out_res.is_file() => {
            println!("cargo:rustc-link-arg-bin=phonecam={}", out_res.display());
        }
        Ok(exit) => {
            println!(
                "cargo:warning=rc.exe exited with status {exit}; skipping embedded Windows executable icon"
            );
        }
        Err(err) => {
            println!(
                "cargo:warning=Failed to invoke rc.exe ({err}); skipping embedded Windows executable icon"
            );
        }
    }
}

fn find_rc_exe() -> Option<PathBuf> {
    if let Ok(path_var) = env::var("PATH") {
        for dir in env::split_paths(&path_var) {
            let candidate = dir.join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    let arch = match env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("x86") => "x86",
        Ok("aarch64") => "arm64",
        _ => "x64",
    };

    let kits_root = Path::new(r"C:\Program Files (x86)\Windows Kits\10\bin");
    let Ok(entries) = std::fs::read_dir(kits_root) else {
        return None;
    };

    let mut versions: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .filter(|p| p.is_dir())
        .collect();
    versions.sort();
    versions.reverse();

    for ver_dir in versions {
        let candidate = ver_dir.join(arch).join("rc.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}
