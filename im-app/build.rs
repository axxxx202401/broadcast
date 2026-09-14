//! im-app 的 Tauri 构建脚本，生成应用清单、权限与平台资源等编译期产物。
//!
//! 根据 `IM_BUNDLE_IDENTIFIER` 环境变量（来自 `.env.<profile>`）在打包时动态修改
//! bundle identifier，使 test / production 两个版本可以共存安装在同一台机器上。

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .capabilities_path_pattern("./src-tauri/capabilities/**/*.json"),
    )
    .unwrap();

    // IM_BUNDLE_IDENTIFIER 为空时（production）跳过修改。
    let suffix = std::env::var("IM_BUNDLE_IDENTIFIER").unwrap_or_default();
    if suffix.is_empty() {
        return;
    }

    let conf_path = std::path::Path::new("tauri.conf.json");
    if !conf_path.exists() {
        return;
    }

    let content = std::fs::read_to_string(conf_path).expect("failed to read tauri.conf.json");
    let modified = content.replace(
        "\"co.68chat.im-monitor\"",
        &format!("\"co.68chat.im-monitor{suffix}\""),
    );
    if modified == content {
        return;
    }
    std::fs::write(conf_path, modified).expect("failed to write tauri.conf.json");
}
