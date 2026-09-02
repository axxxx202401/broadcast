pub mod aes;
pub mod config;
pub mod error;
pub mod tcp_head;
pub mod version_key;

pub const MAX_FRAME_BODY_SIZE: usize = 8 * 1024 * 1024;
pub const MAX_DECOMPRESSED_BODY_SIZE: usize = 32 * 1024 * 1024;

#[cfg(test)]
mod tests;

pub use error::AppError;
