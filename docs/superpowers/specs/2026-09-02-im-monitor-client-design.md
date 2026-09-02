# IM 群消息监控桌面客户端 — 设计文档

> 日期：2026-09-02
> 状态：待评审

## 1. 项目概述

使用 **Tauri v2 + Rust** 开发跨平台桌面客户端，监听指定 IM 群的消息并持久化存储到 SQLite，提供可视化界面。

### 1.1 目标

- 登录 openchat-user 获取 token
- 连接 im-chat TCP 长连接服务接收实时群消息
- 通过 im-biz HTTP 接口拉取群列表，管理监控目标
- 所有群消息存 SQLite，供后续提取/统计使用
- 提供群列表 + 消息流的 UI 界面

### 1.2 已知约束

- im-biz / im-chat 使用 protobuf 二进制协议
- openchat-user 使用 JSON（经 gateway 加密）
- 所有请求通过版本密钥做 AES 加密签名
- 提取规则暂时未知，先全量存储，接口预留

---

## 2. 服务端配置

| 服务 | 地址 | 协议 |
|------|------|------|
| openchat-user（含 gateway） | `https://test-ochat-user1.68chat.co` | HTTP/JSON |
| im-biz | `https://test-biz-b.68chat.co` | HTTP/Protobuf |
| im-chat | `35.220.159.225:9500` | TCP/Protobuf |

### 2.1 版本密钥

| 项 | 值 |
|----|-----|
| version secretName | `f82956caf0fa90aecf24d5ef9541f624` |
| body AES key | `97b1f52761ffc7f8` |
| header AES key | `f58c15f54e8f7826` |
| appVer | 680 |
| packageCode | 9803 |
| plat | 0 (Android) |

### 2.2 设备信息

- sysMac：运行时从系统生成 UUID
- sysModel：`PC-TOOLS`

---

## 3. 整体架构

```
broadcast/                              # Cargo workspace root
├── Cargo.toml                          # workspace manifest
│
├── im-common/                          # 核心类库
│   ├── src/
│   │   ├── lib.rs
│   │   ├── aes.rs                    # AES/ECB/PKCS7 加解密
│   │   ├── tcp_head.rs               # TCP 2字节头解析/构建
│   │   ├── version_key.rs            # X-One 头生成 + 版本密钥管理
│   │   ├── config.rs                 # 配置结构体
│   │   └── error.rs                  # 错误类型
│   └── Cargo.toml
│
├── im-proto/                           # Protobuf binding（prost 生成）
│   ├── src/
│   │   ├── lib.rs                    # 重新导出所有 pb 模块
│   │   └── generated/                # prost 生成代码
│   └── Cargo.toml
│
├── im-http/                            # HTTP 客户端
│   ├── src/
│   │   ├── lib.rs
│   │   ├── openchat_user.rs          # openchat-user JSON API
│   │   ├── im_biz.rs                 # im-biz Protobuf API
│   │   └── client.rs                 # 共享 HTTP 客户端 + 响应解密
│   └── Cargo.toml
│
├── im-chat/                            # TCP 长连接
│   ├── src/
│   │   ├── lib.rs
│   │   ├── client.rs                 # ChatClient 主结构
│   │   ├── frame.rs                  # 帧编码/解码（TCP head + body）
│   │   ├── heartbeat.rs              # 心跳保活
│   │   └── reconnect.rs              # 指数退避重连
│   └── Cargo.toml
│
├── im-store/                           # SQLite 存储
│   ├── src/
│   │   ├── lib.rs
│   │   ├── schema.rs                 # 建表 SQL
│   │   ├── message.rs                # 消息 CRUD
│   │   └── group.rs                  # 群信息 CRUD
│   └── Cargo.toml
│
├── im-app/                             # Tauri v2 桌面应用
│   ├── src/
│   │   ├── main.rs                   # Tauri 入口
│   │   ├── app_state.rs              # 全局状态（token、连接、监控群）
│   │   ├── commands/                 # Tauri commands
│   │   │   ├── auth.rs               # 登录相关
│   │   │   ├── groups.rs             # 群列表/监控管理
│   │   │   └── chat.rs               # 连接控制
│   │   └── monitor.rs                # 后台消息监控任务
│   ├── src-tauri/
│   │   ├── tauri.conf.json           # Tauri 配置
│   │   ├── capabilities/
│   │   ├── tools/
│   │   ├── generated/
│   │   └── build.rs
│   └── Cargo.toml
│
├── docs/
│   └── superpowers/
│       └── specs/
│           └── 2026-09-02-im-monitor-client-design.md
│
└── proto/                              # proto 文件副本（供 prost 编译）
    ├── common.proto
    ├── im.proto
    ├── group.proto
    ├── group_message.proto
    ├── login.proto
    └── ...
```

---

## 4. 模块详细设计

### 4.1 im-common — 加解密 + 版本管理

#### 4.1.1 AES 加密

**算法**：`AES/ECB/PKCS7Padding`，128-bit key

Rust 依赖：`aes` + `cipher` + `pkcs7` crates

```rust
pub struct AesCipher {
    key: Vec<u8>,
}

impl AesCipher {
    /// 加密，返回 raw bytes（与 Java AESHelper.encode 等价）
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, Error>;
    /// 解密，返回 raw bytes（与 Java AESHelper.decode 等价）
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, Error>;
}
```

注意：Java 默认使用 `AES/ECB/PKCS5Padding`，PKCS5 与 PKCS7 在 16 字节块下行为一致，`pkcs7` crate 兼容。

#### 4.1.2 TCP 帧头解析

每个 TCP 消息的 wire 格式：
```
[2 bytes head][2 bytes messageId (big-endian short)][4 bytes contentLength (big-endian int)][content bytes]
```

head 字节定义（来自 `MessageUtil.getRequestHead` + `decodeMessage`）：
```
byte[0] = 0xC0 (1100 0000) — 固定标志位
byte[1] = 0x80 (1000 0000) 未加密
byte[1] = 0xC0 (1100 0000) 已加密或已压缩

bit 位置（从左到右，bit0=最高位）:
  bit0-1  : 固定 11
  bit2-7  : protocolVersion (6 bits)
  bit8    : encrypted       (1 bit)
  bit9    : zipped          (1 bit)
  bit10   : encryptedSystemVersion (1 bit)
  bit11   : isReport        (1 bit)
```

注意：Java `ByteUtil.subBinary(bins, begin, count)` 中 `begin` 是全局 bit 偏移（从 byte[0] bit0 开始计数）。

Rust 实现：
```rust
#[derive(Debug, Clone, Copy)]
pub struct TcpFrameHeader {
    pub protocol_version: u8,    // bits 2-7 of byte[1]，即 (head[1] >> 2) & 0x3F
    pub encrypted: bool,         // bit 8  → (head[1] & 0x04) != 0
    pub zipped: bool,            // bit 9  → (head[1] & 0x02) != 0
    pub encrypted_system_version: bool, // bit 10 → (head[1] & 0x01) != 0
    pub is_report: bool,         // bit 11 → (head[0] & 0x01) != 0  (实际总是 false)
}

impl TcpFrameHeader {
    /// 解析 2 字节 head
    pub fn parse(head: [u8; 2]) -> Self {
        // byte[0] = 0xC0, byte[1] 低6位 = version, bit8-11 在 byte[1] 中
        // 但实际 Java 用 getBinaryFromByte([byte0, byte1]) 得到 16-bit 数组
        // bit0..bit7 = byte[0] 的 bit7..bit0, bit8..bit15 = byte[1] 的 bit7..bit0
        let b0 = head[0]; // 0xC0 = 1100_0000
        let b1 = head[1];
        
        // subBinary(bins, 2, 6) → bits 2-7 of the 16-bit array = b1 的高6位
        let protocol_version = (b1 >> 2) as u8;
        // subBinary(bins, 8, 1) → bit 8 = b1 的 bit6 (0x04)
        let encrypted = (b1 & 0x04) != 0;
        // subBinary(bins, 9, 1) → bit 9 = b1 的 bit7 (0x02)... 等等
        // 实际上 getBinaryFromByte([0xC0, 0x80]):
        //   binary[0..7] = 0xC0 = 1,1,0,0,0,0,0,0
        //   binary[8..15] = 0x80 = 1,0,0,0,0,0,0,0
        // subBinary(bins, 8, 1) = binary[8] = 1 → NOT encrypted? 
        // 但 0x80 表示未加密... 看代码: isEncrypted = !(subBinary(bins, 8, 1) == 0)
        // 0x80 的 bit7=1, 所以 subBinary(bins,8,1)=1, isEncrypted=true? 
        // 不对，看 getRequestHead: byte2=0x80 时 isZip=false
        // 重新分析：getBinaryFromByte(0x80) = [1,0,0,0,0,0,0,0] (bit0=MSB)
        // subBinary(bins, 8, 1) = bins[8] = 1 (0x80 的最高位)
        // isEncrypted = !(1 == 0) = true → 但 0x80 应该未加密!
        //
        // 实际上 getRequestHead 里：isZip=false → byte2=0x80
        // isZip=true → byte2=0xC0
        // 0xC0 = 1100_0000, bit7=1 → 但这里是 isZip 影响 bit7
        // 结论：bit8 对应的是 byte[1] 的 bit7 (0x80)，而不是 0x04
        // 
        // 修正映射（基于 getBinaryFromByte 的 MSB-first 顺序）:
        // binary[8] = head[1] 的 bit7 (0x80) → encrypted
        // binary[9] = head[1] 的 bit6 (0x40) → zipped
        // binary[10] = head[1] 的 bit5 (0x20) → encryptedSystemVersion
        // binary[11] = head[1] 的 bit4 (0x10) → isReport
        // binary[2..8] = head[1] 的 bit3..bit0 → protocolVersion (但只有4位?)
        //
        // 再验证: 0x80 = 1000_0000
        //   encrypted = binary[8]=1 → true... 但 getRequestHead(isZip=false) 返回 0x80
        //   这说明 0x80 = 未加密！
        //   isEncrypted = !(subBinary(bins,8,1)==0) → !(1==0) = true → 矛盾!
        //
        // 重新看：getBinaryFromByte(byte) 中 pos(b, i) = (b & (1<<(8-i-1))) != 0
        // pos(0x80, 0) = (0x80 & 0x80) != 0 = true → binary[0]=true
        // pos(0x80, 1) = (0x80 & 0x40) != 0 = false → binary[1]=false
        // 所以 getBinaryFromByte(0x80) = [true,false,false,false,false,false,false,false]
        // binary[8] = getBinaryFromByte(0x80)[0] = true
        // isEncrypted = !(true == 0) = true... 还是矛盾
        //
        // 看 MessageUtil 里解码端：
        // boolean isEncrypted = !(ByteUtil.subBinary(bins, 8, 1) == 0);
        // 对于 0x80: subBinary([1,0,0,0,0,0,0,0, 1,0,0,0,0,0,0,0], 8, 1) = 1
        // isEncrypted = !(1==0) = true
        // 但 getRequestHead(false) 返回 {0xC0, 0x80} 且 messageContent 被 CipherUtils.encode() 加密了
        // 所以 0x80 = 已加密！那 0xC0 是什么？
        // getRequestHead(true) = {0xC0, 0xC0}
        // 0xC0 = 1100_0000: binary[8]=1(encrypted), binary[9]=1(zipped)
        // 所以：0x80=加密未压缩, 0xC0=加密已压缩
        // 未加密的情况？看 getOldMessage：只用 CipherUtils.encode，不判断加密标志
        // 实际上从未发送过未加密的消息（除了 LoginServer 可能）
        
        // 最终正确映射：
        let encrypted = (b1 & 0x80) != 0;   // bit8  = head[1] & 0x80
        let zipped = (b1 & 0x40) != 0;      // bit9  = head[1] & 0x40
        let encrypted_system_version = (b1 & 0x20) != 0; // bit10
        let is_report = (b1 & 0x10) != 0;   // bit11
        let protocol_version = (b1 & 0x0F) as u8; // bits 12-15 = head[1] 低4位
    }
    
    /// 构建 head：encrypted=true → byte2=0x80, zipped=true → byte2=0xC0
    pub fn build(encrypted: bool, zipped: bool) -> [u8; 2] {
        let mut b1 = 0x00u8;
        if encrypted { b1 |= 0x80; }
        if zipped { b1 |= 0x40; }
        [0xC0, b1]
    }
}
```

#### 4.1.3 X-One 头生成（version key）

im-biz 和 im-chat（加密系统版本时）使用：
- key：header_aes_key = `f58c15f54e8f7826`（16字节）
- 明文格式：`"{secretName},{timestamp_ms}"`
- 加密后转 hex 字符串作为 `X-One` header 值

```rust
pub struct VersionKeyManager {
    secret_name: String,
    header_cipher: AesCipher,
}

impl VersionKeyManager {
    /// 生成 X-One header 值
    pub fn build_x_one(&self) -> String {
        let plaintext = format!("{},{}", self.secret_name, chrono::Utc::now().timestamp_millis());
        let encrypted = self.header_cipher.encrypt(plaintext.as_bytes()).unwrap();
        hex::encode(encrypted)
    }
}
```

**重要**：im-biz 的 `SignitureValidFilter` 中对 `/sys/checkVersionV2` 接口会验证时间戳不超过 1 小时。普通接口只验证 `systemVersion` 不为 null。

#### 4.1.4 配置结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub openchat_user_url: String,   // https://test-ochat-user1.68chat.co
    pub im_biz_url: String,          // https://test-biz-b.68chat.co
    pub im_chat_host: String,        // 35.220.159.225
    pub im_chat_port: u16,           // 9500
    pub version_secret_name: String, // f82956caf0fa90aecf24d5ef9541f624
    pub body_aes_key: String,        // 97b1f52761ffc7f8
    pub header_aes_key: String,      // f58c15f54e8f7826
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    pub app_ver: i32,        // 680
    pub package_code: i32,   // 9803
    pub plat: i32,           // 0 (Android)
    pub language: i32,       // 2 (简体中文)
    pub sys_mac: String,     // 运行时生成 UUID
    pub sys_model: String,   // PC-TOOLS
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub server: ServerConfig,
    pub device: DeviceConfig,
}
```

---

### 4.2 im-proto — Protobuf Binding

使用 `prost` 编译 proto 文件。需要修改 proto 文件的 `java_package`/`java_outer_classname` 选项为 Rust 包名。

#### 4.2.1 Proto 文件清单（需要编译的）

| Proto 文件 | 包含的关键类型 |
|-----------|--------------|
| common.proto | ClientInfo, CommonResult, CommonResultReq, Platform, GroupMemberBase, UserBase |
| im.proto | GroupMessage, MessageType, LoginSessionMessage, PushGroupMessage, ReceiveGroupMessage |
| group.proto | GroupBase, GroupContactListReq, GroupContactListResp |
| group_message.proto | （事件消息相关，暂不用） |
| login.proto | LoginReq（biz 用）, RegReq |
| friend_message.proto | OneToOneMessage（暂不用） |
| channel_event.proto | ChannelEvent（暂不用） |

#### 4.2.2 prost 配置

```toml
# im-proto/Cargo.toml
[dependencies]
prost = "0.13"

[build-dependencies]
prost-build = "0.13"
```

`build.rs` 中配置 include_dir 指向 `../proto/`（symlink 或 copy）。

生成的 Rust 类型示例：
- `common::ClientInfo`
- `im::GroupMessage`
- `im::MessageType`
- `group::GroupBase`
- `login::LoginReq`

---

### 4.3 im-http — HTTP 客户端

#### 4.3.1 openchat-user（JSON 协议）

**Gateway 请求格式**：
- 请求体：gzip + AES(secretKey) 加密的 JSON
- 响应体：2 字节 head（0xC0 0x80/0xC0）+ 4 字节 **big-endian** length + gzip + AES 加密的 JSON

**但 openchat-user 的登录接口 `/sns/login/login` 是直接 JSON**（从 OpenChatLoginServiceTest 看，请求是 JSON，不需要 TCP head）。实际走 gateway，需要确认：
- 请求头：`Content-Type: application/json`
- 请求体：JSON（可能 gzip + AES 加密，需看 gateway RequestBodyParser）
- 响应：从 ResponseEncryptFilter 看，响应是 2 字节 head + 4 字节 length + gzip + AES 加密

**openchat-user 登录流程**（从 OpenChatLoginServiceTest）：

```
1. POST /user/unauthorized/sendSmsCaptchaWithGt4  → 发送短信验证码
2. POST /user/unauthorized/issued                  → 获取 validateToken
3. POST /user/unauthorized/verify                  → 验证验证码
4. POST /sns/login/login                           → 登录，拿到 token
```

login 请求体（JSON）：
```json
{
  "phone": "13800138000",
  "countryCode": 86,
  "loginType": 1,
  "validateToken": "xxx"
}
```

login 响应体（JSON）：
```json
{
  "uid": 779562,
  "isNotLastDeviceMac": false,
  "isLoginOut": 0,
  "oldSessionId": null,
  "token": "eyJ..."
}
```

#### 4.3.2 im-biz（Protobuf 协议）

**请求格式**：
- Header: `X-One: <hex-encoded AES_V_L_SALT(secretName+","+timestamp_ms)>`
- Body: 2 字节 TCP head + 4 字节 **big-endian** length + [gzip] + [AES(secretKey)] 加密的 protobuf `CommonResultReq`

`CommonResultReq` 结构：
```protobuf
message CommonResultReq {
    ClientInfo clientInfo = 1;
}
```

`ClientInfo` 结构：
```protobuf
message ClientInfo {
    string sessionId = 1;
    int32 appVer = 2;
    int32 packageCode = 3;
    Platform plat = 4;
    int32 language = 5;
    string sysMac = 6;
    string sysModel = 7;
    string token = 8;
    string version = 9;
}
```

**关键接口**：
- `POST /group/groupContactList` — 获取我的群聊列表
- `POST /login/login` — biz 侧登录（但主要用 openchat-user 登录）
- `POST /login/getUrls` — 获取服务器 URL 列表

群列表响应 `GroupContactListResp`：
```protobuf
message GroupContactListResp {
    CommonResult commonResult = 1;
    int32 groupCount = 2;
    repeated GroupBase groups = 3;
}

message GroupBase {
    int64 groupId = 1;
    int64 hostId = 2;
    string name = 3;
    string pic = 4;
    bool bfJoinCheck = 5;
    int64 createTime = 6;
    int64 memberCount = 7;
    // ... 其他字段
}
```

#### 4.3.3 响应解密

im-biz 响应格式（从 ResponseEncryptFilter）：
- 2 字节 head（0xC0 + 0x80/0xC0）
- 4 字节 **big-endian** length
- [gzip 解压]
- [AES 解密（用 secretKey）]
- 结果：protobuf bytes（im-biz）或 JSON bytes（openchat-user 经 gateway）

**注意**：openchat-user 通过 gateway，响应经过 ResponseEncryptFilter 加密；im-biz 直接由 SignitureValidFilter + Action 处理，响应也经过加密。两者响应格式相同。

---

### 4.4 im-chat — TCP 长连接

#### 4.4.1 协议帧格式

每个 TCP 消息的 wire 格式（来自 `Message.getMessageBytes()`）：
```
[2 bytes head][2 bytes messageId (big-endian short)][4 bytes contentLength (big-endian int)][content bytes]
```

- head：`[0xC0, 0x80]` 未加密；`[0xC0, 0xC0]` 加密+压缩
- messageId：消息类型 ID（如 1100=登录, 2202=群消息推送）
- contentLength：content bytes 的长度（大端）
- content：加密/压缩后的 protobuf 数据

content 长度指 head 之后的所有内容长度（含 gzip/AES 加密后的 body）。

#### 4.4.2 登录流程

1. 建立 TCP 连接到 `35.220.159.225:9500`
2. 构造 `LoginSessionMessage` protobuf：
   ```protobuf
   message LoginSessionMessage {
       ClientInfo clinetInfo = 1;  // 注意拼写错误是 proto 定义里的
       int64 latestLoginTime = 2;
       string installCode = 3;
       int32 pushTag = 4;
   }
   ```
3. 发送：msgId=1100 (`ClientMessage.LoginServer`)，body = 加密后的 protobuf bytes
4. 接收：msgId=1201 (`ServerMessage.PushLoginSuccess`)，携带 `LoginSessionMessage`

登录成功后，服务端在 session 上设置 `secretKey`，后续消息使用该 key 加密。

#### 4.4.3 消息 ID 路由

```rust
pub enum ChatMessageId {
    HeartBeat = 1000,
    LoginServer = 1100,
    LoginOut = 1101,
    SendGroupMessage = 2101,
    ReceiveGroupMessage = 2102,
    SendRecallGroupMessage = 2103,
    ReceiveRecallGroupMessage = 2104,
    // Server responses
    PushLoginSuccess = 1201,
    PushGroupMessage = 2202,
    PushRecallGroupMessage = 2205,
}
```

#### 4.4.4 群消息处理

收到 `PushGroupMessage`（2202）时：
- 解析 `IMPB.PushGroupMessage` → 得到 `repeated GroupMessage`
- 对每个 `GroupMessage`：
  - 检查是否在监控列表中
  - 调用 `MessageExtractor` 提取内容
  - 存入 SQLite

`GroupMessage` 关键字段：
```protobuf
message GroupMessage {
    int64 sendUid = 1;
    int64 groupId = 2;
    MessageType msgType = 3;   // text=0, image=1, audio=2, video=3, system=6, notice=8...
    bytes content = 4;         // 消息内容（文本时为 UTF-8 字符串）
    int64 sendTime = 7;
    int64 msgId = 8;
    GroupMemberBase sendMember = 9;
    int32 version = 10;
    string contentMd5 = 11;
    string groupName = 13;     // 从 IMPB.GroupMessage 看到
}
```

#### 4.4.5 心跳与重连

- **心跳**：每 2 分钟发送 `HearBeatMessage`（msgId=1000），空 body
- **重连**：连接断开后指数退避，初始 1s，最大 30s，无上限重试
- **保活**：使用 `tokio::time::interval` 定期发送心跳，检测 TCP 层 KeepAlive

---

### 4.5 im-store — SQLite 存储

使用 `sqlx` + `sqlx-lite`（ SQLite 嵌入，无外部依赖）

#### 4.5.1 表结构

```sql
-- 群信息表
CREATE TABLE groups (
    group_id    INTEGER PRIMARY KEY,
    name        TEXT NOT NULL,
    pic         TEXT DEFAULT '',
    host_id     INTEGER,
    member_count INTEGER DEFAULT 0,
    created_at  INTEGER NOT NULL,
    monitored   INTEGER NOT NULL DEFAULT 1,  -- 0=未监控 1=监控中
    updated_at  INTEGER NOT NULL
);

-- 群消息表
CREATE TABLE messages (
    msg_id      INTEGER PRIMARY KEY,
    group_id    INTEGER NOT NULL REFERENCES groups(group_id),
    send_uid    INTEGER NOT NULL,
    msg_type    INTEGER NOT NULL,            -- MessageType enum value
    content     BLOB NOT NULL,               -- 原始 content bytes
    send_time   INTEGER NOT NULL,
    content_md5 TEXT DEFAULT '',
    stored_at   INTEGER NOT NULL,            -- unix timestamp
    raw_proto   BLOB                         -- 完整 GroupMessage 序列化，用于重建
);

CREATE INDEX idx_messages_group_time ON messages(group_id, send_time);
CREATE INDEX idx_messages_group_monitored ON messages(group_id) WHERE group_id IN (SELECT group_id FROM groups WHERE monitored = 1);
```

#### 4.5.2 消息提取接口（预留）

```rust
pub trait MessageExtractor: Send + Sync {
    /// 从消息中提取可用于统计/展示的内容
    fn extract(&self, msg: &GroupMessage) -> ExtractedContent;
}

pub struct ExtractedContent {
    pub text: String,
    pub summary: String,
    pub sender_name: Option<String>,
    pub sender_avatar: Option<String>,
}

/// 默认实现：文本消息直接返回 content，其他类型返回类型名
pub struct DefaultMessageExtractor;
```

---

### 4.6 im-app — Tauri v2 桌面应用

#### 4.6.1 界面布局

```
┌─────────────────────────────────────────────────────┐
│  [Logo]  IM Monitor                    [设置] [退出] │
├──────────────────────┬──────────────────────────────┤
│  搜索群...           │                              │
│                      │  群: 测试群A                  │
│ ┌──────────────────┐ │  ─────────────────────       │
│ │🔵 测试群A [ON]   │ │  14:30 张三: 大家好          │
│ ├──────────────────┤ │  14:31 李四: 收到            │
│ │🔵 交易群 [ON]    │ │  14:32 系统: 新人入群        │
│ ├──────────────────┤ │                              │
│ │🔇 历史群 [OFF]   │ │  [输入框_________________]   │
│ └──────────────────┘ │                              │
│                      │                              │
│ [+ 添加监控群]       │                              │
├──────────────────────┴──────────────────────────────┤
│ 状态: 🔴 未登录  |  🟢 已连接  |  📊 消息: 1,234    │
└─────────────────────────────────────────────────────┘
```

#### 4.6.2 状态管理

```rust
#[derive(Default)]
pub struct AppState {
    pub config: Arc<RwLock<AppConfig>>,
    pub db: Arc<SqliteStore>,
    pub chat_client: Arc<Mutex<Option<ChatClient>>>,
    pub token: Arc<RwLock<Option<String>>>,
    pub uid: Arc<RwLock<Option<i64>>>,
    pub monitoring_groups: Arc<RwLock<HashSet<i64>>>,
}
```

#### 4.6.3 Tauri Commands

```rust
// 认证
#[tauri::command]
async fn send_sms_code(phone: String, country_code: i32) -> Result<SendCodeResult, String>;
#[tauri::command]
async fn get_validate_token() -> Result<String, String>;
#[tauri::command]
async fn verify_code(validate_token: String, code: String) -> Result<(), String>;
#[tauri::command]
async fn login(phone: String, country_code: i32, validate_token: String) -> Result<LoginResult, String>;
#[tauri::command]
async fn logout() -> Result<(), String>;

// 群管理
#[tauri::command]
async fn fetch_group_list() -> Result<Vec<GroupInfo>, String>;
#[tauri::command]
async fn toggle_monitor(group_id: i64, monitored: bool) -> Result<(), String>;

// 连接
#[tauri::command]
async fn connect_chat() -> Result<(), String>;
#[tauri::command]
async fn disconnect_chat() -> Result<(), String>;
```

#### 4.6.4 前端技术栈

- **框架**：Tauri v2 内置前端（无 JS 框架，用原生 HTML + CSS + vanilla JS，或选择 lightweight framework 如 Alpine.js）
- **样式**：CSS variables + 深色/浅色主题
- **状态同步**：Tauri Store plugin 或 Rust state + invoke

#### 4.6.5 后台任务

使用 `tokio::task::spawn` 运行：
1. **ChatMonitorTask**：监听 im-chat 消息，过滤监控群，写入 DB，通过 Tauri event 推送给前端
2. **ReconnectTask**：监测连接状态，断线自动重连

```rust
// 通过 Tauri event 推送新消息给前端
app_handle.emit_all("new_message", &serde_json::json!({
    "group_id": msg.group_id,
    "sender": msg.send_uid,
    "content": String::from_utf8_lossy(&msg.content).to_string(),
    "send_time": msg.send_time,
}))?;
```

---

## 5. 数据流

```
用户输入手机号
    │
    ▼
[im-http::OpenChatUserClient] ──POST /user/unauthorized/sendSmsCaptchaWithGt4──► Gateway
    │
    ▼ 用户输入验证码
[im-http::OpenChatUserClient] ──POST /user/unauthorized/verify + /sns/login/login──► Gateway
    │
    ▼ 拿到 token
[im-http::ImBizClient] ──POST /group/groupContactList──► im-biz
    │
    ▼ 群列表 → UI 显示
[im-chat::ChatClient] ──TCP connect + LoginSessionMessage──► im-chat server
    │
    ▼ 连接成功
[ChatMonitorTask] ──监听 PushGroupMessage(2202)──► 过滤监控群
    │
    ▼
[im-store] ──INSERT messages, groups──► SQLite
    │
    ▼
[Tauri Event: new_message] ──► 前端 UI 渲染
```

---

## 6. 关键技术决策

| 决策点 | 选择 | 理由 |
|--------|------|------|
| Proto 编译 | `prost` + `prost-build` | 轻量、无需 protoc 二进制、Rust native |
| AES 加密 | `aes` + `cipher` + `pkcs7` | 纯 Rust，ECB mode 原生支持 |
| TCP 网络 | `tokio` TcpStream | 异步非阻塞，与 Tauri 事件循环兼容 |
| SQLite | `sqlx` + `sqlx-lite` | 编译期 SQL 检查，零外部依赖 |
| HTTP | `reqwest` + `tokio-util` | 主流异步 HTTP client |
| 压缩 | `flate2`（gzip） | 标准库替代品，支持 Gzip |
| 配置存储 | Tauri Store plugin + JSON 文件 | 跨平台，用户可编辑 |
| IPC | Tauri events（`emit`/`listen`） | 后端→前端实时推送 |

---

## 7. 实施阶段

### Phase 1：核心骨架（本次）
- [ ] 搭建 Cargo workspace
- [ ] 实现 `im-common`：AES 加解密、TCP head 解析、X-One 生成
- [ ] 编译 proto 文件生成 Rust binding
- [ ] 实现 `im-chat` TCP 客户端：连接、登录、心跳、重连、消息接收
- [ ] 实现 `im-store` SQLite 存储
- [ ] 实现 Tauri 主界面框架（群列表 + 消息流）

### Phase 2：HTTP 客户端
- [ ] 实现 `im-http`：openchat-user 登录流程
- [ ] 实现 im-biz 群列表接口
- [ ] 集成登录 → 群列表 → socket 连接流程

### Phase 3：完整功能
- [ ] 前端登录页
- [ ] 监控群管理（开关）
- [ ] 消息实时渲染
- [ ] 配置持久化

### Phase 4：提取规则（后续）
- [ ] 实现 `MessageExtractor` trait
- [ ] 根据业务需求添加具体提取逻辑
- [ ] 统计面板

---

## 8. 安全风险

1. **Token 安全**：token 存储在本地 SQLite/Store，不上传云端；进程内存中尽量缩短持有时间
2. **TLS**：所有 HTTP 通信强制 HTTPS；TCP 连接目前无 TLS（内网/测试环境）
3. **密码/验证码**：不在日志中打印

---

## 9. 待定事项

| 项 | 说明 |
|----|------|
| 极验滑块验证 | 已完成参数配置，前端需集成滑块 UI；Rust 侧透传 gt4DTO JSON |
| im-biz URL 获取 | 登录后可能需要从 `/login/getUrls` 获取最新 im-chat 地址，目前硬编码测试地址 |
| version 动态查询 | 当前使用固定 secretName，生产环境可能需要动态查询最新版本 |

---

## 10. 极验（gt4）配置

| 项 | 值 |
|----|-----|
| captchaId | `0fd8f86d495fa3b8e944c07143e49ced` |
| captchaKey | `4784ce2e73fa19f7be82ed3cf60d3658` |

**前端验证流程**：
1. 用户输入手机号后，前端展示极验滑块验证组件
2. 用户完成拼图后，SDK 返回 `{lotNumber, captchaOutput, passToken, genTime}`
3. 将这些字段填入 `SendSmsCaptchaWithGt4Req.gt4DTO` 发送到服务端
4. 验证成功后才能进行后续登录流程

**Rust 侧只需透传 gt4DTO JSON**，验证 UI 放在 Tauri 前端（HTML/JS）。
