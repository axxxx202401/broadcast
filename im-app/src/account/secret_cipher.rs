//! 本机凭据主密钥与 AES-256-GCM 加解密。
//!
//! 主密钥保存在应用数据目录的 `.credential_key` 中，不调用系统 Keychain。
//! SQLite 中只存 nonce 与 ciphertext，禁止写入 Token 或密码明文。

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand_core::{OsRng, RngCore};
use std::path::Path;

use super::AccountError;

/// AES-256-GCM 主密钥长度（字节）。
pub const MASTER_KEY_LEN: usize = 32;
/// GCM 标准 nonce 长度（字节）。
pub const NONCE_LEN: usize = 12;

/// 读取或生成本机主密钥文件；文件权限在 Unix 上设为 `0600`。
pub async fn load_or_create_master_key(path: &Path) -> Result<[u8; MASTER_KEY_LEN], AccountError> {
    match tokio::fs::read(path).await {
        Ok(bytes) if bytes.len() == MASTER_KEY_LEN => {
            let mut key = [0u8; MASTER_KEY_LEN];
            key.copy_from_slice(&bytes);
            Ok(key)
        }
        Ok(_) => Err(AccountError::CredentialUnavailable(
            "本机密钥文件长度无效".into(),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            create_master_key_file(path).await
        }
        Err(error) => Err(error.into()),
    }
}

/// 生成随机主密钥并写入指定路径；并发创建时回退为读取已有文件。
async fn create_master_key_file(path: &Path) -> Result<[u8; MASTER_KEY_LEN], AccountError> {
    let mut key = [0u8; MASTER_KEY_LEN];
    OsRng.fill_bytes(&mut key);

    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
        {
            Ok(mut file) => {
                file.write_all(&key)?;
                file.sync_all()?;
                return Ok(key);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }

    #[cfg(not(unix))]
    {
        match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await
        {
            Ok(mut file) => {
                use tokio::io::AsyncWriteExt;
                file.write_all(&key).await?;
                file.sync_all().await?;
                return Ok(key);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error.into()),
        }
    }

    match tokio::fs::read(path).await {
        Ok(bytes) if bytes.len() == MASTER_KEY_LEN => {
            let mut existing = [0u8; MASTER_KEY_LEN];
            existing.copy_from_slice(&bytes);
            Ok(existing)
        }
        Ok(_) => Err(AccountError::CredentialUnavailable(
            "本机密钥文件长度无效".into(),
        )),
        Err(error) => Err(error.into()),
    }
}

/// 使用主密钥加密 UTF-8 明文，返回 `(nonce, ciphertext)`。
pub fn encrypt_secret(
    key: &[u8; MASTER_KEY_LEN],
    plaintext: &str,
) -> Result<(Vec<u8>, Vec<u8>), AccountError> {
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AccountError::CredentialUnavailable("加密初始化失败".into()))?;
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|_| AccountError::CredentialUnavailable("凭据加密失败".into()))?;
    Ok((nonce_bytes.to_vec(), ciphertext))
}

/// 使用主密钥解密凭据；失败时只返回类别摘要，不回显明文或密文。
pub fn decrypt_secret(
    key: &[u8; MASTER_KEY_LEN],
    nonce: &[u8],
    ciphertext: &[u8],
) -> Result<String, AccountError> {
    if nonce.len() != NONCE_LEN {
        return Err(AccountError::CredentialUnavailable(
            "凭据 nonce 无效".into(),
        ));
    }
    let cipher = Aes256Gcm::new_from_slice(key)
        .map_err(|_| AccountError::CredentialUnavailable("解密初始化失败".into()))?;
    let nonce = Nonce::from_slice(nonce);
    let plain = cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| AccountError::CredentialUnavailable("凭据解密失败".into()))?;
    String::from_utf8(plain)
        .map_err(|_| AccountError::CredentialUnavailable("凭据明文不是 UTF-8".into()))
}

#[cfg(test)]
mod tests {
    use super::{decrypt_secret, encrypt_secret, load_or_create_master_key, MASTER_KEY_LEN};

    /// 加解密往返不得改变明文，且每次写入使用新的 nonce。
    #[test]
    fn encrypt_decrypt_roundtrip() {
        let key = [7u8; MASTER_KEY_LEN];
        let (nonce_a, cipher_a) = encrypt_secret(&key, "token-value").unwrap();
        let (nonce_b, cipher_b) = encrypt_secret(&key, "token-value").unwrap();
        assert_ne!(nonce_a, nonce_b);
        assert_ne!(cipher_a, cipher_b);
        assert_eq!(
            decrypt_secret(&key, &nonce_a, &cipher_a).unwrap(),
            "token-value"
        );
    }

    /// 主密钥文件只创建一次，后续读取应得到相同内容。
    #[tokio::test]
    async fn master_key_file_is_created_once() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(".credential_key");
        let first = load_or_create_master_key(&path).await.unwrap();
        let second = load_or_create_master_key(&path).await.unwrap();
        assert_eq!(first, second);
        assert_eq!(tokio::fs::metadata(&path).await.unwrap().len(), 32);
    }
}
