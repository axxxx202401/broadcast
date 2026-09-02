fn main() {
    prost_build::Config::new()
        .compile_protos(&["../proto/broadcast.proto"], &["../proto/"])
        .unwrap();
}
