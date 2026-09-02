//! 协议兼容所需的 AES 加解密封装。
//!
//! 本模块固定使用 AES-128-ECB 与 PKCS#7 填充，以匹配既有通信协议；ECB
//! 不提供随机化或消息完整性保证，不应据此推断其适合一般用途。

use aes::Aes128;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit};
use ecb::{Decryptor, Encryptor};

use crate::error::{AppError, AppResult};

/// 使用 AES-128-ECB-PKCS7 的协议兼容加解密器。
///
/// 构造时的密钥必须恰好为 16 字节。
pub struct AesCipher {
    encryptor: Encryptor<Aes128>,
    decryptor: Decryptor<Aes128>,
}

impl AesCipher {
    /// 使用 16 字节密钥创建加解密器。
    ///
    /// # Panics
    ///
    /// 当 `key` 不是 16 字节时 panic。需要处理配置错误的调用方应使用
    /// [`Self::try_new`]。
    pub fn new(key: &[u8]) -> Self {
        Self::try_new(key).expect("AES-128 key must be 16 bytes")
    }

    /// 尝试使用 16 字节密钥创建加解密器。
    ///
    /// 当 `key` 长度不等于 16 字节时返回 [`AppError::Config`]。
    pub fn try_new(key: &[u8]) -> AppResult<Self> {
        let key: [u8; 16] = key.try_into().map_err(|_| {
            AppError::Config(format!("AES-128 key must be 16 bytes, got {}", key.len()))
        })?;
        Ok(Self {
            encryptor: Encryptor::new(&key.into()),
            decryptor: Decryptor::new(&key.into()),
        })
    }

    /// 按 AES-128-ECB-PKCS7 加密明文。
    ///
    /// 本方法预留一个额外分组容纳 PKCS#7 填充，并返回只包含实际密文长度的
    /// 新缓冲区。底层加密失败时返回 [`AppError::AesEncrypt`]。
    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let mut buf = vec![0u8; plaintext.len() + 16];
        let ct = self
            .encryptor
            .clone()
            .encrypt_padded_b2b_mut::<cipher::block_padding::Pkcs7>(plaintext, &mut buf)
            .map_err(|e| AppError::AesEncrypt(e.to_string()))?;
        Ok(ct.to_vec())
    }

    /// 按 AES-128-ECB-PKCS7 解密密文。
    ///
    /// 本方法创建与密文等长的可变缓冲区供解密和去填充使用，并返回只包含实际
    /// 明文长度的新缓冲区。密文长度或 PKCS#7 填充无效时返回
    /// [`AppError::AesDecrypt`]。
    pub fn decrypt(&self, ciphertext: &[u8]) -> AppResult<Vec<u8>> {
        let mut buf = ciphertext.to_vec();
        let pt = self
            .decryptor
            .clone()
            .decrypt_padded_b2b_mut::<cipher::block_padding::Pkcs7>(ciphertext, &mut buf)
            .map_err(|e| AppError::AesDecrypt(e.to_string()))?;
        Ok(pt.to_vec())
    }
}
