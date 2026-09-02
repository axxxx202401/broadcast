use aes::Aes128;
use cipher::{BlockDecryptMut, BlockEncryptMut, KeyInit};
use ecb::{Decryptor, Encryptor};

use crate::error::{AppError, AppResult};

pub struct AesCipher {
    encryptor: Encryptor<Aes128>,
    decryptor: Decryptor<Aes128>,
}

impl AesCipher {
    pub fn new(key: &[u8]) -> Self {
        Self::try_new(key).expect("AES-128 key must be 16 bytes")
    }

    pub fn try_new(key: &[u8]) -> AppResult<Self> {
        let key: [u8; 16] = key.try_into().map_err(|_| {
            AppError::Config(format!("AES-128 key must be 16 bytes, got {}", key.len()))
        })?;
        Ok(Self {
            encryptor: Encryptor::new(&key.into()),
            decryptor: Decryptor::new(&key.into()),
        })
    }

    pub fn encrypt(&self, plaintext: &[u8]) -> AppResult<Vec<u8>> {
        let mut buf = vec![0u8; plaintext.len() + 16];
        let ct = self
            .encryptor
            .clone()
            .encrypt_padded_b2b_mut::<cipher::block_padding::Pkcs7>(plaintext, &mut buf)
            .map_err(|e| AppError::AesEncrypt(e.to_string()))?;
        Ok(ct.to_vec())
    }

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
