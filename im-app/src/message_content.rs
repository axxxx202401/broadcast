//! 群消息正文与附件的端到端解密。

use aes::Aes128;
use cipher::{generic_array::GenericArray, BlockDecrypt, KeyInit};
use im_common::aes::AesCipher;
use prost::Message;
use serde::Serialize;
use std::{
    collections::HashMap,
    hash::Hash,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Weak,
    },
};
use x25519_dalek::{PublicKey, StaticSecret};

const PLAIN_ATTACHMENT_CHUNK: usize = 102_400;
const CIPHER_ATTACHMENT_CHUNK: usize = 102_416;

/// 生成尚未由服务端分配版本的当前用户 X25519 App 密钥对。
pub fn generate_user_key_pair(uid: i64) -> im_store::key_pair::UserKeyPairRecord {
    let private = StaticSecret::random_from_rng(rand_core::OsRng);
    let public = PublicKey::from(&private);
    im_store::key_pair::UserKeyPairRecord {
        uid,
        key_version: 0,
        public_key: hex::encode_upper(public.as_bytes()),
        private_key: hex::encode_upper(private.to_bytes()),
    }
}

/// 判断本地密钥是否与 1201 公布的当前 App 公钥和版本完全对应。
pub fn should_reuse_user_key_pair(
    local: &im_store::key_pair::UserKeyPairRecord,
    server: &im_proto::KeyPairBase,
) -> bool {
    local.key_version > 0
        && local.key_version == server.key_version
        && local.public_key.eq_ignore_ascii_case(&server.public_key)
        && !local.private_key.is_empty()
}

/// 恢复与 1201 匹配的本地私钥，或生成新密钥并通过调用方登记公钥。
///
/// 登记成功后才使用服务端版本持久化完整密钥对并安装到内存。登记、SQLite 写入任一步
/// 失败均返回错误；调用方应把它作为消息解密故障处理，而不是 TCP 登录失败。
pub async fn synchronize_user_key_pair<Register, RegisterFuture>(
    message_crypto: &MessageCryptoState,
    key_pair_store: &im_store::key_pair::UserKeyPairStore,
    uid: i64,
    server_key_pair: &im_proto::KeyPairBase,
    register: Register,
) -> Result<(), String>
where
    Register: FnOnce(String) -> RegisterFuture,
    RegisterFuture: std::future::Future<Output = Result<i32, String>>,
{
    if let Some(local) = key_pair_store
        .get_latest(uid)
        .await
        .map_err(|error| format!("读取本地用户密钥失败：{error}"))?
        .filter(|local| should_reuse_user_key_pair(local, server_key_pair))
    {
        message_crypto
            .install_own_private_key(local.private_key)
            .await;
        return Ok(());
    }

    let mut generated = generate_user_key_pair(uid);
    generated.key_version = register(generated.public_key.clone()).await?;
    key_pair_store
        .set(&generated)
        .await
        .map_err(|error| format!("保存本地用户密钥失败：{error}"))?;
    message_crypto
        .install_own_private_key(generated.private_key)
        .await;
    Ok(())
}

/// 前端可以直接消费的消息正文。
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MediaContent {
    /// 文本消息。
    Text {
        /// 文本正文。
        text: String,
    },
    /// 图片消息。
    Image {
        /// 原图地址。
        url: String,
        /// 缩略图地址。
        thumbnail_url: String,
        /// 文件字节数。
        file_size: i64,
        /// 图片宽度。
        width: i32,
        /// 图片高度。
        height: i32,
    },
    /// 音频消息。
    Audio {
        /// 密文附件地址。
        url: String,
        /// 时长，单位由服务端协议定义。
        duration: i32,
        /// 文件字节数。
        file_size: i64,
    },
    /// 视频消息。
    Video {
        /// 视频密文地址。
        url: String,
        /// 缩略图密文地址。
        thumbnail_url: String,
        /// 时长，单位由服务端协议定义。
        duration: i32,
        /// 文件字节数。
        file_size: i64,
        /// 视频宽度。
        width: i32,
        /// 视频高度。
        height: i32,
    },
    /// 文件消息。
    File {
        /// 密文附件地址。
        url: String,
        /// 原始文件名。
        name: String,
        /// MIME 类型。
        mime_type: String,
        /// 文件字节数。
        file_size: i64,
    },
}

/// 一项可下载附件的远端定位和保存提示。
pub struct AttachmentDescriptor {
    /// OSS 或 CDN 密文地址。
    pub url: String,
    /// 用于生成本地缓存文件名的展示名称。
    pub name: String,
    /// 媒体类型；协议缺失时使用宽泛类型。
    pub mime_type: String,
}

impl MediaContent {
    /// 选择主附件或缩略图；文本消息返回 `None`。
    pub fn attachment(&self, thumbnail: bool) -> Option<AttachmentDescriptor> {
        match self {
            Self::Text { .. } => None,
            Self::Image {
                url, thumbnail_url, ..
            } => Some(AttachmentDescriptor {
                url: if thumbnail && !thumbnail_url.is_empty() {
                    thumbnail_url.clone()
                } else {
                    url.clone()
                },
                name: if thumbnail { "thumbnail" } else { "image" }.to_string(),
                mime_type: "image/*".to_string(),
            }),
            Self::Audio { url, .. } => Some(AttachmentDescriptor {
                url: url.clone(),
                name: "audio".to_string(),
                mime_type: "audio/*".to_string(),
            }),
            Self::Video {
                url, thumbnail_url, ..
            } => Some(AttachmentDescriptor {
                url: if thumbnail && !thumbnail_url.is_empty() {
                    thumbnail_url.clone()
                } else {
                    url.clone()
                },
                name: if thumbnail {
                    "video-thumbnail"
                } else {
                    "video"
                }
                .to_string(),
                mime_type: if thumbnail { "image/*" } else { "video/*" }.to_string(),
            }),
            Self::File {
                url,
                name,
                mime_type,
                ..
            } => Some(AttachmentDescriptor {
                url: url.clone(),
                name: name.clone(),
                mime_type: mime_type.clone(),
            }),
        }
    }
}

/// 当前登录会话用于解密群消息的内存密钥状态。
///
/// 用户私钥和派生群密钥不会写入 SQLite，也不会序列化到前端。新登录安装用户私钥时
/// 会清空上一会话的群密钥缓存，防止跨账号复用密钥。
#[derive(Default)]
pub struct MessageCryptoState {
    own_private_key: tokio::sync::RwLock<Option<String>>,
    group_rel_keys: KeyedSingleflightCache<(i64, i32), String>,
}

/// 按 key 隔离的异步单飞缓存。
///
/// 快路径只读值缓存；未命中时，同 key 请求共享一把异步锁并在取得锁后再次检查缓存，
/// 不同 key 不互相阻塞。锁表保存 `Weak`，最后一个等待者结束后删除条目，避免会话内群组
/// 数持续增长。`clear` 推进代次并清空值；正在执行的旧代 loader 可以返回给原调用方，
/// 但不得把结果写入新会话缓存，活动锁则保留到持有者和等待者自然退出。
struct KeyedSingleflightCache<K, V> {
    values: tokio::sync::RwLock<HashMap<K, (u64, V)>>,
    locks: std::sync::Mutex<HashMap<K, Weak<tokio::sync::Mutex<()>>>>,
    generation: AtomicU64,
}

impl<K, V> Default for KeyedSingleflightCache<K, V> {
    fn default() -> Self {
        Self {
            values: tokio::sync::RwLock::new(HashMap::new()),
            locks: std::sync::Mutex::new(HashMap::new()),
            generation: AtomicU64::new(0),
        }
    }
}

impl<K, V> KeyedSingleflightCache<K, V>
where
    K: Clone + Eq + Hash,
    V: Clone,
{
    /// 返回缓存值，或在当前 key 的单飞锁内执行一次 loader；错误不会进入缓存。
    async fn get_or_try_init<Load, LoadFuture>(&self, key: K, load: Load) -> Result<V, String>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: std::future::Future<Output = Result<V, String>>,
    {
        self.get_or_try_init_with_store_hooks(key, load, || async {}, || async {})
            .await
    }

    /// 在写缓存前及持有值写锁后分别执行测试钩子，用于覆盖 clear 的两侧临界交错。
    async fn get_or_try_init_with_store_hooks<
        Load,
        LoadFuture,
        BeforeStore,
        BeforeStoreFuture,
        AfterCheck,
        AfterCheckFuture,
    >(
        &self,
        key: K,
        load: Load,
        before_store: BeforeStore,
        after_check: AfterCheck,
    ) -> Result<V, String>
    where
        Load: FnOnce() -> LoadFuture,
        LoadFuture: std::future::Future<Output = Result<V, String>>,
        BeforeStore: FnOnce() -> BeforeStoreFuture,
        BeforeStoreFuture: std::future::Future<Output = ()>,
        AfterCheck: FnOnce() -> AfterCheckFuture,
        AfterCheckFuture: std::future::Future<Output = ()>,
    {
        let fast_generation = self.generation.load(Ordering::Acquire);
        let values = self.values.read().await;
        if let Some((value_generation, value)) = values.get(&key) {
            // 第二次读取 generation 是 fast path 的线性化点；clear 已推进代次时旧值立即失效。
            if *value_generation == fast_generation
                && self.generation.load(Ordering::Acquire) == fast_generation
            {
                return Ok(value.clone());
            }
        }
        drop(values);
        let key_lock = {
            let mut locks = self.locks.lock().expect("singleflight lock map poisoned");
            locks.retain(|_, weak| weak.strong_count() > 0);
            if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        let guard = key_lock.lock().await;
        let load_generation = self.generation.load(Ordering::Acquire);
        let values = self.values.read().await;
        if let Some((value_generation, value)) = values.get(&key) {
            if *value_generation == load_generation
                && self.generation.load(Ordering::Acquire) == load_generation
            {
                return Ok(value.clone());
            }
        }
        drop(values);

        let result = load().await;
        before_store().await;
        if let Ok(value) = &result {
            let mut values = self.values.write().await;
            let generation_matched_on_entry =
                self.generation.load(Ordering::Acquire) == load_generation;
            after_check().await;
            // generation 检查与 insert 同处写锁临界区；clear 先推进 generation，因此无论
            // 双方等待写锁的先后，旧 loader 都不可能生成新代可见的缓存项。
            if generation_matched_on_entry
                && self.generation.load(Ordering::Acquire) == load_generation
            {
                values.insert(key.clone(), (load_generation, value.clone()));
            }
        }
        drop(guard);

        let mut locks = self.locks.lock().expect("singleflight lock map poisoned");
        if Arc::strong_count(&key_lock) == 1 {
            locks.remove(&key);
        }
        result
    }

    /// 清空值缓存并使执行中的旧代 loader 失去回填资格；只移除已无活动任务的锁。
    async fn clear(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        self.values.write().await.clear();
        self.locks
            .lock()
            .expect("singleflight lock map poisoned")
            .retain(|_, weak| weak.strong_count() > 0);
    }
}

impl MessageCryptoState {
    /// 安装调用方提供的当前用户私钥并清空派生缓存。
    pub async fn install_own_private_key(&self, private_key: String) {
        *self.own_private_key.write().await = Some(private_key);
        self.group_rel_keys.clear().await;
    }

    /// 清除当前用户及所有群组的内存密钥。
    pub async fn clear(&self) {
        *self.own_private_key.write().await = None;
        self.group_rel_keys.clear().await;
    }

    /// 测试辅助：当前是否仍持有用户私钥。
    #[cfg(test)]
    pub(crate) async fn has_own_private_key(&self) -> bool {
        self.own_private_key.read().await.is_some()
    }

    /// 解密一条群消息并解析五种受支持正文，同时返回附件本体密钥。
    ///
    /// `version == 0` 时正文和附件密钥按明文协议处理；其他版本按群 ID 与版本缓存
    /// 派生密钥，缓存未命中时调用 `/sys/getKeyPair`。任何失败均作为可展示错误返回，
    /// 调用方仍可保留原始消息并发送已入库回执。
    pub async fn decode_group_message(
        &self,
        http: &im_http::im_biz::ImBizClient,
        client_info: &im_proto::ClientInfo,
        message: &im_proto::GroupMessage,
    ) -> Result<DecodedMessageContent, String> {
        let (plain, file_key) = if message.version == 0 {
            (
                message.content.clone(),
                (!message.attachment_key.is_empty()).then(|| message.attachment_key.clone()),
            )
        } else {
            let rel_key = self
                // OCS 仅把 message.version 当作“正文已加密”标志，群密钥接口固定请求版本 1。
                .group_rel_key(http, client_info, message.group_id, 1)
                .await?;
            let plain = decrypt_group_value(&rel_key, &message.content)?;
            let file_key = if message.attachment_key.is_empty() {
                None
            } else {
                let encrypted = hex::decode(&message.attachment_key)
                    .map_err(|error| format!("attachmentKey 不是 HEX：{error}"))?;
                Some(
                    String::from_utf8(decrypt_group_value(&rel_key, &encrypted)?)
                        .map_err(|error| format!("附件密钥不是 UTF-8：{error}"))?
                        .trim()
                        .to_string(),
                )
            };
            (plain, file_key)
        };

        Ok(DecodedMessageContent {
            content: decode_media_content(message.msg_type, &plain)?,
            file_key,
        })
    }

    async fn group_rel_key(
        &self,
        http: &im_http::im_biz::ImBizClient,
        client_info: &im_proto::ClientInfo,
        group_id: i64,
        version: i32,
    ) -> Result<String, String> {
        self.group_rel_keys
            .get_or_try_init((group_id, version), || async {
                let private_key = self
                    .own_private_key
                    .read()
                    .await
                    .clone()
                    .filter(|value| !value.is_empty())
                    .ok_or_else(|| "本地 App 密钥尚未就绪，暂时无法解包群消息密钥".to_string())?;
                let key_pair = http
                    .fetch_group_key_pair(client_info, group_id)
                    .await
                    .map_err(|error| format!("获取群密钥失败：{error}"))?;
                decrypt_group_rel_key(&private_key, &key_pair.public_key, &key_pair.msg_key)
            })
            .await
    }
}

/// 一条消息的结构化正文和仅供 Rust 附件下载使用的密钥。
pub struct DecodedMessageContent {
    /// 可序列化到前端的结构化正文。
    pub content: MediaContent,
    /// 解密 OSS 附件所需密钥；文本消息通常没有该值。
    pub file_key: Option<String>,
}

/// 通过当前用户私钥和群公钥解开服务端包装的群消息密钥。
///
/// 三个输入均遵循 PC 客户端协议：Curve25519 密钥和包装密文使用十六进制，
/// ECDH 共享值转为大写十六进制后取前 16 个 UTF-8 字节作为 AES key。
pub fn decrypt_group_rel_key(
    own_private_key_hex: &str,
    group_public_key_hex: &str,
    encrypted_message_key_hex: &str,
) -> Result<String, String> {
    let private_key = decode_fixed_hex::<32>("用户私钥", own_private_key_hex)?;
    let public_key = decode_fixed_hex::<32>("群公钥", group_public_key_hex)?;
    let encrypted_message_key = hex::decode(encrypted_message_key_hex)
        .map_err(|error| format!("群消息密钥不是 HEX：{error}"))?;
    let shared = StaticSecret::from(private_key).diffie_hellman(&PublicKey::from(public_key));
    let shared_hex = hex::encode_upper(shared.as_bytes());
    let plain = decrypt_group_value(&shared_hex, &encrypted_message_key)?;
    String::from_utf8(plain)
        .map(|value| value.trim().to_string())
        .map_err(|error| format!("群消息密钥不是 UTF-8：{error}"))
}

/// 使用业务密钥前 16 个 UTF-8 字节解开消息正文或 `attachmentKey`。
pub fn decrypt_group_value(key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let key = key
        .as_bytes()
        .get(..16)
        .ok_or_else(|| "业务密钥不足 16 字节".to_string())?;
    AesCipher::try_new(key)
        .and_then(|cipher| cipher.decrypt(ciphertext))
        .map_err(|error| error.to_string())
}

/// 按消息类型将解密后的 Protobuf 正文转换成结构化模型。
pub fn decode_media_content(msg_type: i32, content: &[u8]) -> Result<MediaContent, String> {
    match msg_type {
        0 => im_proto::TextObj::decode(content)
            .map(|value| MediaContent::Text {
                text: value.content,
            })
            .map_err(|error| format!("文本正文解码失败：{error}")),
        1 => im_proto::ImageObj::decode(content)
            .map(|value| MediaContent::Image {
                url: value.url,
                thumbnail_url: value.thumb_url,
                file_size: value.file_size,
                width: value.width,
                height: value.height,
            })
            .map_err(|error| format!("图片正文解码失败：{error}")),
        2 => im_proto::AudioObj::decode(content)
            .map(|value| MediaContent::Audio {
                url: value.url,
                duration: value.duration,
                file_size: value.file_size,
            })
            .map_err(|error| format!("音频正文解码失败：{error}")),
        3 => im_proto::VideoObj::decode(content)
            .map(|value| MediaContent::Video {
                url: value.url,
                thumbnail_url: value.thumb_url,
                duration: value.duration,
                file_size: value.file_size,
                width: value.width,
                height: value.height,
            })
            .map_err(|error| format!("视频正文解码失败：{error}")),
        7 => im_proto::FileObj::decode(content)
            .map(|value| MediaContent::File {
                url: value.file_url,
                name: value.name,
                mime_type: value.mime_type,
                file_size: value.size,
            })
            .map_err(|error| format!("文件正文解码失败：{error}")),
        _ => Err(format!("暂不支持消息类型 {msg_type}")),
    }
}

/// 解密 PC 分块或移动端整文件附件。
///
/// PC 每 102400 字节明文独立做 PKCS7，因此完整密文块固定为 102416 字节；
/// 移动端整文件只在末尾填充。函数检查第一块末尾独立 AES block 是否为完整填充块，
/// 再决定逐块或整文件解密，避免大文件错位。
pub fn decrypt_attachment_bytes(file_key: &str, ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    let key = file_key
        .as_bytes()
        .get(..16)
        .ok_or_else(|| "附件密钥不足 16 字节".to_string())?;
    let cipher = AesCipher::try_new(key).map_err(|error| error.to_string())?;

    if ciphertext.len() > CIPHER_ATTACHMENT_CHUNK && has_pc_chunk_padding(key, ciphertext) {
        let mut plain = Vec::with_capacity(ciphertext.len());
        for chunk in ciphertext.chunks(CIPHER_ATTACHMENT_CHUNK) {
            plain.extend(cipher.decrypt(chunk).map_err(|error| error.to_string())?);
        }
        Ok(plain)
    } else {
        cipher
            .decrypt(ciphertext)
            .map_err(|error| error.to_string())
    }
}

fn has_pc_chunk_padding(key: &[u8], ciphertext: &[u8]) -> bool {
    let Some(block) = ciphertext.get(PLAIN_ATTACHMENT_CHUNK..CIPHER_ATTACHMENT_CHUNK) else {
        return false;
    };
    let Ok(cipher) = Aes128::new_from_slice(key) else {
        return false;
    };
    let mut block = GenericArray::clone_from_slice(block);
    cipher.decrypt_block(&mut block);
    block.iter().all(|byte| *byte == 16)
}

fn decode_fixed_hex<const N: usize>(name: &str, value: &str) -> Result<[u8; N], String> {
    let bytes = hex::decode(value).map_err(|error| format!("{name}不是 HEX：{error}"))?;
    bytes
        .try_into()
        .map_err(|bytes: Vec<u8>| format!("{name}必须为 {N} 字节，实际为 {} 字节", bytes.len()))
}

#[cfg(test)]
mod tests {
    use prost::Message;

    use super::{
        decode_fixed_hex, decode_media_content, decrypt_group_rel_key, decrypt_group_value,
        generate_user_key_pair, should_reuse_user_key_pair, synchronize_user_key_pair,
        KeyedSingleflightCache, MediaContent, MessageCryptoState,
    };

    #[tokio::test]
    async fn keyed_singleflight_runs_one_loader_for_eight_same_key_requests() {
        let cache = std::sync::Arc::new(KeyedSingleflightCache::<i64, String>::default());
        let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let tasks = (0..8).map(|_| {
            let cache = cache.clone();
            let loads = loads.clone();
            tokio::spawn(async move {
                cache
                    .get_or_try_init(7, || async move {
                        loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                        Ok("shared-key".to_string())
                    })
                    .await
            })
        });

        let results = futures::future::join_all(tasks).await;

        assert!(results
            .into_iter()
            .all(|result| result.unwrap().as_deref() == Ok("shared-key")));
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn keyed_singleflight_allows_different_keys_to_load_concurrently() {
        let cache = std::sync::Arc::new(KeyedSingleflightCache::<i64, String>::default());
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(2));
        let tasks = [7_i64, 8_i64].map(|key| {
            let cache = cache.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                cache
                    .get_or_try_init(key, || async move {
                        barrier.wait().await;
                        Ok(format!("key-{key}"))
                    })
                    .await
            })
        });

        let results = tokio::time::timeout(
            std::time::Duration::from_millis(200),
            futures::future::join_all(tasks),
        )
        .await
        .expect("不同 key 不应被同一把锁串行阻塞");

        assert_eq!(results[0].as_ref().unwrap().as_deref(), Ok("key-7"));
        assert_eq!(results[1].as_ref().unwrap().as_deref(), Ok("key-8"));
    }

    #[tokio::test]
    async fn keyed_singleflight_does_not_cache_errors_and_allows_retry() {
        let cache = KeyedSingleflightCache::<i64, String>::default();
        let loads = std::sync::atomic::AtomicUsize::new(0);

        let first = cache
            .get_or_try_init(7, || async {
                loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err("temporary".to_string())
            })
            .await;
        let second = cache
            .get_or_try_init(7, || async {
                loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("recovered".to_string())
            })
            .await;
        let third = cache
            .get_or_try_init(7, || async {
                loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("unexpected".to_string())
            })
            .await;

        assert_eq!(first.unwrap_err(), "temporary");
        assert_eq!(second.as_deref(), Ok("recovered"));
        assert_eq!(third.as_deref(), Ok("recovered"));
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn clear_between_old_loader_completion_and_insert_prevents_stale_refill() {
        let cache = std::sync::Arc::new(KeyedSingleflightCache::<i64, String>::default());
        let loader_finished = std::sync::Arc::new(tokio::sync::Notify::new());
        let allow_insert = std::sync::Arc::new(tokio::sync::Notify::new());
        let old_task = {
            let cache = cache.clone();
            let loader_finished = loader_finished.clone();
            let allow_insert = allow_insert.clone();
            tokio::spawn(async move {
                cache
                    .get_or_try_init_with_store_hooks(
                        7,
                        || async { Ok("old".to_string()) },
                        || async {},
                        || async move {
                            loader_finished.notify_one();
                            allow_insert.notified().await;
                        },
                    )
                    .await
            })
        };
        loader_finished.notified().await;

        let clear_task = {
            let cache = cache.clone();
            tokio::spawn(async move { cache.clear().await })
        };
        while cache.generation.load(std::sync::atomic::Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
        allow_insert.notify_one();
        assert_eq!(old_task.await.unwrap().as_deref(), Ok("old"));
        clear_task.await.unwrap();

        let loads = std::sync::atomic::AtomicUsize::new(0);
        let current = cache
            .get_or_try_init(7, || async {
                loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok("new".to_string())
            })
            .await
            .unwrap();
        assert_eq!(current, "new");
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn fast_path_rejects_old_generation_while_clear_has_not_physically_drained_map() {
        let cache = std::sync::Arc::new(KeyedSingleflightCache::<i64, String>::default());
        cache
            .get_or_try_init(7, || async { Ok("old".to_string()) })
            .await
            .unwrap();
        // 持有写锁，让 clear 只能先推进逻辑代次、再等待物理清表；随后发起的 fast path
        // 必须等待并拒绝旧代缓存，不能因为旧条目仍在 map 中而返回 "old"。
        let values_guard = cache.values.write().await;
        let clear_task = {
            let cache = cache.clone();
            tokio::spawn(async move { cache.clear().await })
        };
        while cache.generation.load(std::sync::atomic::Ordering::Acquire) == 0 {
            tokio::task::yield_now().await;
        }
        let loads = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let fast_task = {
            let cache = cache.clone();
            let loads = loads.clone();
            tokio::spawn(async move {
                cache
                    .get_or_try_init(7, || async move {
                        loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        Ok("new".to_string())
                    })
                    .await
            })
        };
        drop(values_guard);
        let during_clear = fast_task.await.unwrap().unwrap();
        assert_eq!(during_clear, "new");
        assert_eq!(loads.load(std::sync::atomic::Ordering::SeqCst), 1);
        clear_task.await.unwrap();
    }

    #[test]
    fn decrypts_group_rel_key_with_curve25519_material() {
        let own_private_key = "77076D0A7318A57D3C16C17251B26645DF4C2F87EBC0992AB177FBA51DB92C2A";
        let group_public_key = "DE9EDB7D7B7DC1B4D35B61C2ECE435373F8343C85B78674DADFC7E146F882B4F";
        let encrypted_message_key =
            "FD6E0CED8D8C37C9E3E8D70DC5175F6BA879EE3577C98C9C0270B55EF4930CDE";

        let key = decrypt_group_rel_key(own_private_key, group_public_key, encrypted_message_key)
            .expect("应解出群消息密钥");

        assert_eq!(key, "1234567890123456");
    }

    #[test]
    fn generated_user_key_pair_contains_matching_x25519_material() {
        let generated = generate_user_key_pair(42);
        let private = decode_fixed_hex::<32>("private", &generated.private_key).unwrap();
        let expected_public =
            x25519_dalek::PublicKey::from(&x25519_dalek::StaticSecret::from(private));

        assert_eq!(generated.uid, 42);
        assert_eq!(generated.key_version, 0);
        assert_eq!(
            hex::decode(generated.public_key).unwrap(),
            expected_public.as_bytes()
        );
    }

    #[test]
    fn only_matching_server_version_and_public_key_reuses_local_private_key() {
        let local = im_store::key_pair::UserKeyPairRecord {
            uid: 42,
            key_version: 3,
            public_key: "ABCDEF".to_string(),
            private_key: "private".to_string(),
        };
        let matching_server = im_proto::KeyPairBase {
            public_key: "abcdef".to_string(),
            key_version: 3,
            ..Default::default()
        };

        assert!(should_reuse_user_key_pair(&local, &matching_server));
        assert!(!should_reuse_user_key_pair(
            &local,
            &im_proto::KeyPairBase {
                public_key: "different".to_string(),
                key_version: 3,
                ..Default::default()
            }
        ));
    }

    #[tokio::test]
    async fn synchronization_restores_matching_local_private_key_without_registration() {
        let store = im_store::SqliteStore::new("sqlite::memory:").await.unwrap();
        let local = im_store::key_pair::UserKeyPairRecord {
            uid: 42,
            key_version: 3,
            public_key: "ABCDEF".to_string(),
            private_key: "private".to_string(),
        };
        store.key_pairs.set(&local).await.unwrap();
        let crypto = MessageCryptoState::default();

        synchronize_user_key_pair(
            &crypto,
            &store.key_pairs,
            42,
            &im_proto::KeyPairBase {
                public_key: "abcdef".to_string(),
                key_version: 3,
                ..Default::default()
            },
            |_| async { Err("不应调用登记接口".to_string()) },
        )
        .await
        .unwrap();

        assert_eq!(
            crypto.own_private_key.read().await.as_deref(),
            Some("private")
        );
    }

    #[tokio::test]
    async fn synchronization_generates_registers_and_persists_missing_key_pair() {
        let store = im_store::SqliteStore::new("sqlite::memory:").await.unwrap();
        let crypto = MessageCryptoState::default();

        synchronize_user_key_pair(
            &crypto,
            &store.key_pairs,
            42,
            &im_proto::KeyPairBase::default(),
            |public_key| async move {
                assert_eq!(hex::decode(public_key).unwrap().len(), 32);
                Ok(8)
            },
        )
        .await
        .unwrap();

        let stored = store.key_pairs.get_latest(42).await.unwrap().unwrap();
        assert_eq!(stored.key_version, 8);
        assert_eq!(
            crypto.own_private_key.read().await.as_deref(),
            Some(stored.private_key.as_str())
        );
    }

    #[test]
    fn decrypts_group_value_with_rel_key_prefix() {
        let encrypted =
            hex::decode("7AD5CCE9290B2B8052F644145769A493D9F94D9096AA5E671527C93A1EEA21B2")
                .unwrap();

        let plain = decrypt_group_value("1234567890123456-extra", &encrypted)
            .expect("应使用前 16 字节解密");

        assert_eq!(plain, b"hello monitored group");
    }

    #[test]
    fn decodes_supported_media_protobuf_payloads() {
        let image = im_proto::ImageObj {
            width: 800,
            height: 600,
            file_size: 1234,
            url: "https://cdn.test/image.enc".into(),
            thumb_url: "https://cdn.test/thumb.enc".into(),
            ..Default::default()
        }
        .encode_to_vec();
        let audio = im_proto::AudioObj {
            duration: 12,
            file_size: 2345,
            url: "https://cdn.test/audio.enc".into(),
            ..Default::default()
        }
        .encode_to_vec();
        let video = im_proto::VideoObj {
            width: 1920,
            height: 1080,
            file_size: 3456,
            url: "https://cdn.test/video.enc".into(),
            thumb_url: "https://cdn.test/video-thumb.enc".into(),
            duration: 34,
            ..Default::default()
        }
        .encode_to_vec();
        let file = im_proto::FileObj {
            size: 4567,
            file_url: "https://cdn.test/file.enc".into(),
            name: "report.pdf".into(),
            mime_type: "application/pdf".into(),
            ..Default::default()
        }
        .encode_to_vec();

        assert!(matches!(
            decode_media_content(1, &image).unwrap(),
            MediaContent::Image { width: 800, .. }
        ));
        assert!(matches!(
            decode_media_content(2, &audio).unwrap(),
            MediaContent::Audio { duration: 12, .. }
        ));
        assert!(matches!(
            decode_media_content(3, &video).unwrap(),
            MediaContent::Video { duration: 34, .. }
        ));
        assert!(matches!(
            decode_media_content(7, &file).unwrap(),
            MediaContent::File { ref name, .. } if name == "report.pdf"
        ));
    }

    #[test]
    fn decrypts_pc_chunked_and_whole_file_schemes() {
        let key = "1234567890123456";
        let short_plain = b"small encrypted attachment";
        let short_cipher = im_common::aes::AesCipher::try_new(key.as_bytes())
            .unwrap()
            .encrypt(short_plain)
            .unwrap();
        assert_eq!(
            super::decrypt_attachment_bytes(key, &short_cipher).unwrap(),
            short_plain
        );

        let first_plain = vec![b'a'; 102_400];
        let second_plain = vec![b'b'; 71];
        let cipher = im_common::aes::AesCipher::try_new(key.as_bytes()).unwrap();
        let mut chunked = cipher.encrypt(&first_plain).unwrap();
        chunked.extend(cipher.encrypt(&second_plain).unwrap());

        let mut expected = first_plain;
        expected.extend(second_plain);
        assert_eq!(
            super::decrypt_attachment_bytes(key, &chunked).unwrap(),
            expected
        );
    }

    #[test]
    fn selects_thumbnail_and_primary_attachment_urls() {
        let image = MediaContent::Image {
            url: "https://cdn.test/image.enc".into(),
            thumbnail_url: "https://cdn.test/thumb.enc".into(),
            file_size: 1,
            width: 2,
            height: 3,
        };

        assert_eq!(
            image.attachment(true).unwrap().url,
            "https://cdn.test/thumb.enc"
        );
        assert_eq!(
            image.attachment(false).unwrap().url,
            "https://cdn.test/image.enc"
        );
        assert!(MediaContent::Text {
            text: "hello".into()
        }
        .attachment(false)
        .is_none());
    }
}
