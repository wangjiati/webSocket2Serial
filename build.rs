//! Windows 资源编译：把应用图标与托盘状态图标嵌入 exe。
//!
//! 仅在目标为 Windows MSVC 时调用 Windows SDK 自带的 `rc.exe`；其他目标或
//! 找不到资源编译器时跳过，不影响正常构建。

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

const ASSETS: [&str; 5] = [
    "src/assets/icon.ico",
    "src/assets/tray-idle.ico",
    "src/assets/tray-active.ico",
    "src/assets/tray-waiting.ico",
    "src/assets/tray-error.ico",
];

fn main() {
    println!("cargo:rerun-if-changed=src/assets/app.rc");
    for asset in ASSETS {
        println!("cargo:rerun-if-changed={asset}");
    }

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows")
        || env::var("CARGO_CFG_TARGET_ENV").as_deref() != Ok("msvc")
    {
        return;
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let assets_dir = manifest_dir.join("src").join("assets");
    let out_dir = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR"));
    let resource = out_dir.join("websocket2serial.res");

    let Some(rc) = find_rc() else {
        println!("cargo:warning=未找到 rc.exe，跳过 Windows 图标资源");
        return;
    };

    let status = Command::new(&rc)
        .current_dir(&assets_dir)
        .arg("/nologo")
        .arg("/fo")
        .arg(&resource)
        .arg("app.rc")
        .status();

    match status {
        Ok(status) if status.success() && resource.is_file() => {
            println!("cargo:rustc-link-arg-bins={}", resource.display());
        }
        Ok(status) => println!(
            "cargo:warning=rc.exe 退出码 {:?}，跳过 Windows 图标资源",
            status.code()
        ),
        Err(err) => println!("cargo:warning=无法执行 rc.exe（{err}），跳过 Windows 图标资源"),
    }
}

/// 查找 Windows SDK 的 `rc.exe`；优先使用 `RC` 环境变量。
fn find_rc() -> Option<PathBuf> {
    if let Ok(explicit) = env::var("RC") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Some(path);
        }
    }

    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_default();
    let arch = match target_arch.as_str() {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" | "i586" | "i686" => "x86",
        _ => return None,
    };

    let mut roots = Vec::new();
    if let Ok(sdk) = env::var("WindowsSdkDir") {
        roots.push(PathBuf::from(sdk).join("bin"));
    }
    for key in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Ok(program_files) = env::var(key) {
            roots.push(
                PathBuf::from(program_files)
                    .join("Windows Kits")
                    .join("10")
                    .join("bin"),
            );
        }
    }
    roots.push(PathBuf::from(r"C:\Program Files (x86)\Windows Kits\10\bin"));

    for root in roots {
        let Ok(entries) = fs::read_dir(&root) else {
            continue;
        };
        let mut versions = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect::<Vec<_>>();
        versions.sort();
        versions.reverse();
        for version in versions {
            let candidate = version.join(arch).join("rc.exe");
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
