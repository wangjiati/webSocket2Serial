//! 交付物打包检查：参考客户端 / CLI 示例 / 协议文档的存在性、回环地址、
//! 关键动作与「无外部 CDN」，安装脚本关键内容，以及可选依赖（Python `--help`、
//! Node `--check`、Python `py_compile`、Bash `-n`）。

use std::process::Command;

/// 参考客户端与 CLI 示例必须存在，并固定回环地址与关键动作。
#[test]
fn client_and_cli_exist_and_use_loopback() {
    let js = include_str!("../clients/ws2serial-client.js");
    assert!(js.contains("127.0.0.1"), "参考客户端必须固定 127.0.0.1");
    assert!(
        js.contains("session.open"),
        "参考客户端必须使用 session.open"
    );
    assert!(
        js.contains("arraybuffer"),
        "数据 WS 必须 binaryType=arraybuffer"
    );

    let py = include_str!("../examples/ws2serial-cli.py");
    assert!(py.contains("--port"), "CLI 必须提供 --port");
    assert!(py.contains("127.0.0.1"), "CLI 必须固定 127.0.0.1");

    let doc = include_str!("../docs/protocol-v1.md");
    assert!(
        doc.contains("session.open"),
        "协议文档必须覆盖 session.open"
    );
    assert!(doc.contains("127.0.0.1"), "协议文档必须固定 127.0.0.1");
}

/// 参考客户端与 CLI 示例不得引入外部 CDN（无构建、纯本地交付）。
#[test]
fn deliverables_have_no_external_cdn() {
    for (name, text) in [
        (
            "clients/ws2serial-client.js",
            include_str!("../clients/ws2serial-client.js"),
        ),
        (
            "examples/ws2serial-cli.py",
            include_str!("../examples/ws2serial-cli.py"),
        ),
        (
            "docs/protocol-v1.md",
            include_str!("../docs/protocol-v1.md"),
        ),
    ] {
        for marker in ["cdn.", "unpkg", "jsdelivr", "https://cdn"] {
            assert!(
                !text.contains(marker),
                "{name} 不应引用外部 CDN（发现 {marker}）"
            );
        }
    }
}

/// CLI 的 `--help` 在未安装 `websockets` 时也必须可用（惰性 import）。
/// 环境中没有 `python`/`python3` 时跳过，不视为失败。
#[test]
fn cli_help_exits_zero_without_websockets() {
    let Some(python) = find_command(&["python", "python3", "py"]) else {
        eprintln!("跳过：环境未找到 python/python3");
        return;
    };
    let output = Command::new(python)
        .args(["examples/ws2serial-cli.py", "--help"])
        .output()
        .expect("执行 python --help 失败");
    assert!(
        output.status.success(),
        "CLI --help 应退出 0，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("--port"), "--help 应列出 --port");
}

/// 若环境有 Node，用 `node --check` 校验参考客户端语法；无 Node 则跳过。
#[test]
fn client_js_parses_with_node() {
    let Some(node) = find_command(&["node"]) else {
        eprintln!("跳过：环境未找到 node");
        return;
    };
    let output = Command::new(node)
        .args(["--check", "clients/ws2serial-client.js"])
        .output()
        .expect("执行 node --check 失败");
    assert!(
        output.status.success(),
        "node --check 应通过，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 安装脚本必须生成 Linux autostart `.desktop` 自启项，且引用二进制名。
#[test]
fn install_script_mentions_desktop_and_binary() {
    let s = include_str!("../packaging/install.sh");
    assert!(s.contains(".config/autostart"));
    assert!(s.contains("websocket2serial"));
}

/// 打包脚本必须引用固定工具链与静态 CRT 目标，产出 zip/tar.gz。
#[test]
fn package_scripts_reference_toolchain_and_artifacts() {
    let ps1 = include_str!("../scripts/package-windows.ps1");
    assert!(ps1.contains("cargo +1.77.2 build --release"));
    assert!(ps1.contains("x86_64-pc-windows-msvc"));
    assert!(ps1.contains("crt-static"));
    assert!(ps1.to_ascii_lowercase().contains(".zip"));

    let sh = include_str!("../scripts/package-linux.sh");
    assert!(sh.contains("ubuntu:18.04"));
    assert!(sh.to_ascii_lowercase().contains(".tar.gz"));
}

/// soak.py 提供 `--hours` 与 `--reconnects`，且默认不硬编码 8 小时长跑。
#[test]
fn soak_script_has_hours_and_reconnects_short_default() {
    let py = include_str!("../scripts/soak.py");
    assert!(py.contains("--hours"));
    assert!(py.contains("--reconnects"));
    // 默认短跑：默认时长必须明显小于 8 小时（防止验收脚本默认跑整天）。
    assert!(!py.contains("8 * 60 * 60"));
    assert!(!py.contains("28800"));
}

/// 若环境有 Python，用 `py_compile` 校验 soak.py 语法；无则跳过。
#[test]
fn soak_script_py_compiles() {
    let Some(python) = find_command(&["python", "python3", "py"]) else {
        eprintln!("跳过：环境未找到 python/python3");
        return;
    };
    let output = Command::new(python)
        .args(["-m", "py_compile", "scripts/soak.py"])
        .output()
        .expect("执行 py_compile 失败");
    assert!(
        output.status.success(),
        "soak.py 语法错误，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 若环境有 Bash，用 `bash -n` 校验 install.sh 语法；无则跳过。
#[test]
fn install_script_bash_parses() {
    let Some(bash) = find_command(&["bash"]) else {
        eprintln!("跳过：环境未找到 bash");
        return;
    };
    let output = Command::new(bash)
        .args(["-n", "packaging/install.sh"])
        .output()
        .expect("执行 bash -n 失败");
    assert!(
        output.status.success(),
        "install.sh 语法错误，stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// 在 PATH 中查找首个可执行命令，找不到返回 `None`。
///
/// 只有 `--version` 实际成功退出（`status.success()`）才算找到，避免把
/// Windows Store 的 python 占位 stub（能启动但非零退出）误判为可用。
fn find_command(candidates: &[&str]) -> Option<String> {
    candidates.iter().map(|c| c.to_string()).find(|c| {
        Command::new(c)
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    })
}
