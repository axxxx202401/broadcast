use std::io::{Read, Write};
use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};

use flate2::{read::GzDecoder, write::GzEncoder, Compression};
use im_common::{aes::AesCipher, config::AppConfig};
use im_proto::LoginSessionMessage;
use prost::Message;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use super::{
    client::build_login_frame,
    client::ChatClient,
    frame::{
        decode_frame, decode_server_frame, decode_transport_frame, encode_frame,
        encode_transport_frame, FrameDecodeError, MAX_DECOMPRESSED_BODY_SIZE, MAX_FRAME_BODY_SIZE,
        PRE_SESSION_AES_KEY,
    },
    heartbeat::{heartbeat_loop, heartbeat_message, HEARTBEAT_MSG_ID},
    reconnect::{reconnect_loop, ExponentialBackoff},
};

#[test]
fn test_encode_and_decode_frame() {
    let content = b"test protobuf data";
    let framed = encode_frame(1000, content, false, false).unwrap();

    // Frame should start with [0xC0, 0x00] (encrypted=false, zipped=false)
    assert_eq!(&framed[0..2], &[0xC0, 0x00]);

    let (msg_id, body) = decode_frame(&framed).unwrap();
    assert_eq!(msg_id, 1000);
    assert_eq!(body, content);
}

#[test]
fn test_encode_frame_big_endian() {
    let content = b"hello";
    let framed = encode_frame(0x0102, content, false, false).unwrap();

    // messageId 0x0102 should be big-endian: [0x01, 0x02]
    assert_eq!(&framed[2..4], &[0x01, 0x02]);

    // contentLength 5 should be big-endian: [0x00, 0x00, 0x00, 0x05]
    assert_eq!(&framed[4..8], &[0x00, 0x00, 0x00, 0x05]);
}

#[test]
fn test_decode_invalid_frame() {
    let invalid = vec![0xFF, 0xFF, 0xFF];
    assert!(decode_frame(&invalid).is_err());
}

// --- Integration tests ---

#[test]
fn test_full_frame_workflow() {
    let config = AppConfig::default();
    let key = AesCipher::new(config.server.body_aes_key.as_bytes());

    // 1. 加密数据
    let plaintext = b"test protobuf content";
    let encrypted = key.encrypt(plaintext).unwrap();

    // 2. 编码帧
    let frame = encode_frame(2202, &encrypted, true, false).unwrap();

    // 3. 解码帧
    let (msg_id, decrypted) = decode_frame(&frame).unwrap();
    assert_eq!(msg_id, 2202);

    // 4. 解密
    let result = key.decrypt(&decrypted).unwrap();
    assert_eq!(result, plaintext);
}

#[test]
fn encrypted_transport_frame_never_marks_plaintext_as_encrypted() {
    let config = AppConfig::default();
    let plaintext = b"sensitive protobuf payload";

    let frame =
        encode_transport_frame(&config.server.body_aes_key, 2202, plaintext, true, false).unwrap();

    assert_eq!(&frame[..2], &[0xC0, 0x80]);
    assert_ne!(&frame[8..], plaintext);
    let cipher = AesCipher::new(config.server.body_aes_key.as_bytes());
    assert_eq!(cipher.decrypt(&frame[8..]).unwrap(), plaintext);
}

#[test]
fn transport_encoding_matches_java_encrypt_then_gzip_order() {
    let config = AppConfig::default();
    let plaintext = b"repeated payload repeated payload repeated payload";

    let frame =
        encode_transport_frame(&config.server.body_aes_key, 2202, plaintext, true, true).unwrap();

    assert_eq!(&frame[..2], &[0xC0, 0xC0]);
    let mut decoder = GzDecoder::new(&frame[8..]);
    let mut encrypted = Vec::new();
    decoder.read_to_end(&mut encrypted).unwrap();
    let cipher = AesCipher::new(config.server.body_aes_key.as_bytes());
    assert_eq!(cipher.decrypt(&encrypted).unwrap(), plaintext);
}

#[test]
fn transport_decoding_accepts_java_gzip_wrapped_ciphertext() {
    let config = AppConfig::default();
    let plaintext = b"incoming protobuf payload incoming protobuf payload";
    let cipher = AesCipher::new(config.server.body_aes_key.as_bytes());
    let encrypted = cipher.encrypt(plaintext).unwrap();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(&encrypted).unwrap();
    let frame = encode_frame(2202, &encoder.finish().unwrap(), true, true).unwrap();

    let decoded = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap();

    assert_eq!(decoded.message_id, 2202);
    assert_eq!(decoded.content, plaintext);
    assert_eq!(decoded.wire_len, frame.len());
    assert!(decoded.header.encrypted);
    assert!(decoded.header.zipped);
}

#[test]
fn truncated_transport_frame_is_incomplete() {
    let config = AppConfig::default();
    let mut frame = encode_frame(2202, b"complete payload", false, false).unwrap();
    frame.truncate(frame.len() - 3);

    let error = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap_err();

    assert!(matches!(error, FrameDecodeError::Incomplete { .. }));
}

#[test]
fn complete_bad_encrypted_frame_is_invalid() {
    let config = AppConfig::default();
    let frame = encode_frame(2202, b"not-valid-aes", true, false).unwrap();

    let error = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap_err();

    assert!(matches!(error, FrameDecodeError::Invalid(_)));
}

#[test]
fn encrypted_empty_connection_ack_is_not_decrypted() {
    let config = AppConfig::default();
    let frame = encode_frame(200, &[], true, false).unwrap();

    let decoded = decode_server_frame(&config.server.body_aes_key, &frame).unwrap();

    assert_eq!(decoded.message_id, 200);
    assert!(decoded.content.is_empty());
}

#[test]
fn pre_session_error_frame_uses_java_default_key() {
    let config = AppConfig::default();
    let error = im_proto::ErrrMessage {
        error_msg_code: 4003,
        error_msg: "permission denied".to_string(),
        message_protocol_id: 1100,
        ..Default::default()
    };
    let frame = encode_transport_frame(
        PRE_SESSION_AES_KEY,
        9999,
        &error.encode_to_vec(),
        true,
        false,
    )
    .unwrap();

    let decoded = decode_server_frame(&config.server.body_aes_key, &frame).unwrap();
    let decoded_error = im_proto::ErrrMessage::decode(decoded.content.as_slice()).unwrap();

    assert_eq!(decoded.message_id, 9999);
    assert_eq!(decoded_error.error_msg_code, 4003);
    assert_eq!(decoded_error.error_msg, "permission denied");
}

#[test]
fn invalid_transport_aes_key_returns_error_instead_of_panicking() {
    let frame = encode_frame(2202, b"not-valid-aes", true, false).unwrap();

    let error = decode_transport_frame("short-key", &frame).unwrap_err();

    assert!(matches!(error, FrameDecodeError::Invalid(_)));
}

#[test]
fn oversized_declared_frame_is_invalid_not_incomplete() {
    let config = AppConfig::default();
    let mut frame = vec![0xC0, 0x00, 0x08, 0x9A];
    frame.extend_from_slice(&((MAX_FRAME_BODY_SIZE + 1) as u32).to_be_bytes());

    let error = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap_err();

    assert!(matches!(error, FrameDecodeError::Invalid(_)));
}

#[test]
fn encode_frame_rejects_body_over_limit() {
    let oversized = vec![0u8; MAX_FRAME_BODY_SIZE + 1];

    let error = encode_frame(2202, &oversized, false, false).unwrap_err();

    assert!(matches!(error, im_common::error::AppError::TcpFrame(_)));
}

#[test]
fn transport_decode_rejects_gzip_output_over_limit() {
    let config = AppConfig::default();
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    let chunk = [0u8; 8192];
    let mut remaining = MAX_DECOMPRESSED_BODY_SIZE + 1;
    while remaining > 0 {
        let count = remaining.min(chunk.len());
        encoder.write_all(&chunk[..count]).unwrap();
        remaining -= count;
    }
    let compressed = encoder.finish().unwrap();
    let frame = encode_frame(2202, &compressed, false, true).unwrap();

    let error = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap_err();

    assert!(matches!(error, FrameDecodeError::Invalid(_)));
}

#[test]
fn malformed_gzip_is_reported_as_protocol_error() {
    let config = AppConfig::default();
    let frame = encode_frame(2202, b"not-a-gzip-stream", false, true).unwrap();

    let error = decode_transport_frame(&config.server.body_aes_key, &frame).unwrap_err();

    assert!(matches!(
        error,
        FrameDecodeError::Invalid(im_common::error::AppError::TcpFrame(_))
    ));
}

#[tokio::test]
async fn invalid_frame_disconnects_shared_stream_and_prevents_send() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let disconnected = Arc::new(AtomicBool::new(false));
    let observed = disconnected.clone();
    let mut client = ChatClient::new(config);
    client.on_disconnect(move || {
        let observed = observed.clone();
        async move {
            observed.store(true, Ordering::SeqCst);
        }
    });

    client.connect().await.unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let invalid = encode_frame(2202, b"not-valid-aes", true, false).unwrap();
        socket.write_all(&invalid).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        while !disconnected.load(Ordering::SeqCst) {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();

    let error = client.send(1000, b"").await.unwrap_err();
    assert!(error.to_string().contains("Not connected"));
    server.abort();
}

#[tokio::test]
async fn active_disconnect_stops_reader_before_returning() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let dispatch_count = Arc::new(AtomicUsize::new(0));
    let observed = dispatch_count.clone();
    let mut client = ChatClient::new(config);
    client.on_message(move |_, _| {
        let observed = observed.clone();
        async move {
            observed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    });
    let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();

    client.connect().await.unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        accepted_tx.send(()).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        let frame = encode_frame(2202, b"late message", false, false).unwrap();
        let _ = socket.write_all(&frame).await;
    });
    accepted_rx.await.unwrap();

    client.disconnect().await;
    server.await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;

    assert_eq!(dispatch_count.load(Ordering::SeqCst), 0);
    assert!(client.send(1000, b"").await.is_err());
}

#[tokio::test]
async fn duplicate_connect_returns_already_connected_error() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let mut client = ChatClient::new(config);
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    client.connect().await.unwrap();
    let error = client.connect().await.unwrap_err();

    assert!(matches!(
        error.downcast_ref::<im_common::error::AppError>(),
        Some(im_common::error::AppError::AlreadyConnected)
    ));
    client.disconnect().await;
    server.abort();
}

#[tokio::test]
async fn reconnect_waits_until_old_disconnect_notification_finishes() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let notification_started = Arc::new(tokio::sync::Notify::new());
    let release_notification = Arc::new(tokio::sync::Notify::new());
    let started = notification_started.clone();
    let release = release_notification.clone();
    let mut client = ChatClient::new(config);
    client.on_disconnect(move || {
        let started = started.clone();
        let release = release.clone();
        async move {
            started.notify_one();
            release.notified().await;
        }
    });
    let server = tokio::spawn(async move {
        let (mut first, _) = listener.accept().await.unwrap();
        let invalid = encode_frame(2202, b"not-valid-aes", true, false).unwrap();
        first.write_all(&invalid).await.unwrap();
        let (_second, _) = listener.accept().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    });

    client.connect().await.unwrap();
    notification_started.notified().await;
    let stale_error = client.connect().await.unwrap_err();
    assert!(matches!(
        stale_error.downcast_ref::<im_common::error::AppError>(),
        Some(im_common::error::AppError::AlreadyConnected)
    ));

    release_notification.notify_one();
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            match client.connect().await {
                Ok(()) => break,
                Err(error)
                    if matches!(
                        error.downcast_ref::<im_common::error::AppError>(),
                        Some(im_common::error::AppError::AlreadyConnected)
                    ) =>
                {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("unexpected reconnect error: {error}"),
            }
        }
    })
    .await
    .unwrap();

    release_notification.notify_one();
    client.disconnect().await;
    server.abort();
}

#[test]
fn login_frame_encrypts_protobuf_before_setting_encrypted_flag() {
    let config = AppConfig::default();

    let frame = build_login_frame(&config, "login-token", 42).unwrap();

    assert_eq!(frame[0], 0xC0);
    assert_eq!(frame[1] & 0xA0, 0xA0);
    let x_one_len = u32::from_be_bytes(frame[8..12].try_into().unwrap()) as usize;
    let x_one = std::str::from_utf8(&frame[12..12 + x_one_len]).unwrap();
    let header_cipher = AesCipher::new(config.server.header_aes_key.as_bytes());
    let x_one_plaintext = header_cipher.decrypt(&hex::decode(x_one).unwrap()).unwrap();
    assert!(std::str::from_utf8(&x_one_plaintext)
        .unwrap()
        .starts_with(&format!("{},", config.server.version_secret_name)));

    let wire_payload = &frame[12 + x_one_len..];
    let encrypted = if frame[1] & 0x40 != 0 {
        let mut decoder = GzDecoder::new(wire_payload);
        let mut decoded = Vec::new();
        decoder.read_to_end(&mut decoded).unwrap();
        decoded
    } else {
        wire_payload.to_vec()
    };
    let cipher = AesCipher::new(config.server.body_aes_key.as_bytes());
    let plaintext = cipher.decrypt(&encrypted).unwrap();
    let login = LoginSessionMessage::decode(plaintext.as_slice()).unwrap();
    assert_eq!(login.clinet_info.unwrap().token, "login-token");
}

#[tokio::test]
async fn dropping_connected_client_closes_socket_and_aborts_reader() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut byte = [0u8; 1];
        socket.read(&mut byte).await.unwrap()
    });
    let mut client = ChatClient::new(config);
    client.connect().await.unwrap();

    drop(client);

    assert_eq!(
        tokio::time::timeout(std::time::Duration::from_secs(1), server)
            .await
            .unwrap()
            .unwrap(),
        0
    );
}

#[test]
fn heartbeat_is_one_empty_application_payload() {
    let (message_id, payload) = heartbeat_message();

    assert_eq!(message_id, HEARTBEAT_MSG_ID);
    assert!(payload.is_empty());
}

#[tokio::test(start_paused = true)]
async fn heartbeat_waits_one_full_period_before_first_send_and_is_cancellable() {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let sends = Arc::new(AtomicUsize::new(0));
    let observed = sends.clone();
    let task_cancellation = cancellation.clone();
    let task = tokio::spawn(async move {
        heartbeat_loop(
            std::time::Duration::from_secs(120),
            task_cancellation,
            move || {
                let observed = observed.clone();
                async move {
                    observed.fetch_add(1, Ordering::SeqCst);
                    Ok::<(), ()>(())
                }
            },
        )
        .await
    });

    tokio::task::yield_now().await;
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    tokio::time::advance(std::time::Duration::from_secs(119)).await;
    tokio::task::yield_now().await;
    assert_eq!(sends.load(Ordering::SeqCst), 0);
    tokio::time::advance(std::time::Duration::from_secs(1)).await;
    tokio::task::yield_now().await;
    assert_eq!(sends.load(Ordering::SeqCst), 1);

    cancellation.cancel();
    tokio::time::advance(std::time::Duration::from_secs(240)).await;
    assert_eq!(task.await.unwrap(), Ok(()));
    assert_eq!(sends.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn cancellable_sender_never_holds_client_slot_while_waiting_for_writer() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut config = AppConfig::default();
    config.server.im_chat_host = address.ip().to_string();
    config.server.im_chat_port = address.port();
    let server = tokio::spawn(async move {
        let (_socket, _) = listener.accept().await.unwrap();
        std::future::pending::<()>().await;
    });
    let mut client = ChatClient::new(config);
    client.connect().await.unwrap();
    let sender = client.sender().expect("connected client has a sender");
    let writer_guard = sender.stream.lock().await;
    let slot = Arc::new(tokio::sync::Mutex::new(Some(client)));
    let cancellation = tokio_util::sync::CancellationToken::new();
    let send_cancellation = cancellation.clone();
    let send = tokio::spawn({
        let sender = sender.clone();
        async move {
            sender
                .send_cancellable(
                    HEARTBEAT_MSG_ID,
                    b"",
                    &send_cancellation,
                    std::time::Duration::from_secs(30),
                )
                .await
        }
    });
    tokio::task::yield_now().await;

    let mut client = tokio::time::timeout(std::time::Duration::from_millis(50), async {
        slot.lock().await.take().unwrap()
    })
    .await
    .expect("heartbeat send must not hold the client slot");
    cancellation.cancel();
    let error = tokio::time::timeout(std::time::Duration::from_millis(50), send)
        .await
        .expect("cancellation must interrupt a writer-lock wait")
        .unwrap()
        .unwrap_err();
    assert!(error.to_string().contains("cancelled"));

    drop(writer_guard);
    client.disconnect().await;
    server.abort();
}

#[test]
fn reconnect_backoff_doubles_and_caps_at_thirty_seconds() {
    let delays = ExponentialBackoff::default().take(8).collect::<Vec<_>>();

    assert_eq!(
        delays,
        [1, 2, 4, 8, 16, 30, 30, 30].map(std::time::Duration::from_secs)
    );
}

#[tokio::test]
async fn reconnect_uses_injected_wait_and_connect_login_action_until_success() {
    let cancellation = tokio_util::sync::CancellationToken::new();
    let delays = Arc::new(std::sync::Mutex::new(Vec::new()));
    let observed_delays = delays.clone();
    let attempts = Arc::new(AtomicUsize::new(0));
    let observed_attempts = attempts.clone();

    let result = reconnect_loop(
        cancellation,
        ExponentialBackoff::default(),
        move || {
            let observed_attempts = observed_attempts.clone();
            async move {
                let attempt = observed_attempts.fetch_add(1, Ordering::SeqCst);
                if attempt < 2 {
                    Err("connect or login failed")
                } else {
                    Ok("connected")
                }
            }
        },
        move |delay| {
            observed_delays.lock().unwrap().push(delay);
            std::future::ready(())
        },
    )
    .await;

    assert_eq!(result, Some("connected"));
    assert_eq!(attempts.load(Ordering::SeqCst), 3);
    assert_eq!(
        *delays.lock().unwrap(),
        [1, 2, 4].map(std::time::Duration::from_secs)
    );
}

#[test]
fn test_version_key_generation() {
    use im_common::version_key::VersionKeyManager;
    let manager = VersionKeyManager::new(
        "f82956caf0fa90aecf24d5ef9541f624".to_string(),
        "f58c15f54e8f7826".to_string(),
    );
    let x_one = manager.build_x_one().unwrap();
    assert_eq!(x_one.len(), 96);
    // 验证 hex 解码成功
    hex::decode(&x_one).unwrap();
}
