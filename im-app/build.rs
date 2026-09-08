//! im-app 的 Tauri 构建脚本，生成应用清单、权限与平台资源等编译期产物。

fn main() {
    tauri_build::try_build(
        tauri_build::Attributes::new()
            .capabilities_path_pattern("./src-tauri/capabilities/**/*.json"),
    )
    .unwrap();
}
