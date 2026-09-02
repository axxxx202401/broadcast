//! `im-proto` 的 protobuf 构建入口。
//!
//! Cargo 执行本脚本时，`prost-build` 会读取共享协议文件
//! `../proto/broadcast.proto`，并将生成的 Rust 类型写入 `OUT_DIR`。

fn main() {
    // 协议文件变化后必须重新运行构建脚本，避免继续使用过期的生成代码。
    println!("cargo:rerun-if-changed=../proto/broadcast.proto");
    prost_build::Config::new()
        .compile_protos(&["../proto/broadcast.proto"], &["../proto/"])
        .unwrap();
}
