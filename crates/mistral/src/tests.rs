use std::time::Duration;

use base64::Engine;
use finance_core::ai::{BackendError, Generate};
use finance_secrets::Secret;
use serde_json::{Value, json};

use crate::fake::{chat, serve};
use crate::{Chat, Mistral, valid_key};

fn client(url: &str) -> Mistral {
    Mistral::local(Secret::new("test-key".into()), url, Duration::from_secs(5))
        .unwrap()
        .attempts(1)
}

#[test]
fn chat_asks_for_json_with_the_key() {
    let (url, server) = serve(vec![(200, chat(r#"{"ok":true}"#))]);
    let answer = Chat {
        client: &client(&url),
        model: "mistral-small-latest",
    }
    .generate("the prompt", Some("the system"))
    .unwrap();
    assert_eq!(answer, r#"{"ok":true}"#);
    let seen = server.join().unwrap();
    assert_eq!(seen[0].path, "/v1/chat/completions");
    assert_eq!(seen[0].authorization, "Bearer test-key");
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["model"], "mistral-small-latest");
    assert_eq!(body["response_format"]["type"], "json_object");
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["content"], "the prompt");
}

#[test]
fn a_rejected_key_is_a_status() {
    let (url, server) = serve(vec![(401, "{}".into())]);
    let error = client(&url).chat_json("m", None, "p", 0.0).unwrap_err();
    assert_eq!(error, BackendError::Status(401));
    server.join().unwrap();
}

#[test]
fn embeddings_come_back_in_input_order() {
    let answer = json!({"data": [
        {"index": 1, "embedding": [0.0, 1.0]},
        {"index": 0, "embedding": [1.0, 0.0]},
    ]});
    let (url, server) = serve(vec![(200, answer.to_string())]);
    let vectors = client(&url).embeddings(&["a".into(), "b".into()]).unwrap();
    assert_eq!(vectors, vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    let body: Value = serde_json::from_str(&server.join().unwrap()[0].body).unwrap();
    assert_eq!(body["model"], "mistral-embed");
}

#[test]
fn speech_decodes_the_audio() {
    let audio = base64::engine::general_purpose::STANDARD.encode(b"ID3fake");
    let (url, server) = serve(vec![(200, json!({ "audio_data": audio }).to_string())]);
    let bytes = client(&url)
        .speech("Hola, Gisel here.", Some("v-1"), Some(1.43))
        .unwrap();
    assert_eq!(bytes, b"ID3fake");
    let seen = server.join().unwrap();
    assert_eq!(seen[0].path, "/v1/audio/speech");
    let body: Value = serde_json::from_str(&seen[0].body).unwrap();
    assert_eq!(body["voice_id"], "v-1");
    assert_eq!(body["response_format"], "mp3");
    assert_eq!(body["model"], "voxtral-mini-tts-3-0");
    assert_eq!(body["expressiveness"], 1.4);
    assert_eq!(body["input"], "Hola, Gisel here.");
}

#[test]
fn speech_refuses_empty_text() {
    let mistral = client("http://127.0.0.1:9");
    assert!(mistral.speech("  ", None, None).is_err());
}

#[test]
fn refused_expressiveness_is_dropped_and_tried_again() {
    let audio = base64::engine::general_purpose::STANDARD.encode(b"ID3");
    let (url, server) = serve(vec![
        (400, "{}".into()),
        (200, json!({ "audio_data": audio }).to_string()),
    ]);
    let bytes = client(&url)
        .speech("Hola.", Some("voice_3143"), Some(1.6))
        .unwrap();
    assert_eq!(bytes, b"ID3");
    let seen = server.join().unwrap();
    let first: Value = serde_json::from_str(&seen[0].body).unwrap();
    let second: Value = serde_json::from_str(&seen[1].body).unwrap();
    assert_eq!(first["expressiveness"], 1.6);
    assert!(second.get("expressiveness").is_none());
}

#[test]
fn voices_are_read() {
    let answer = json!({"data": [
        {"id": "a1", "slug": "paul_neutral", "name": "Paul", "languages": ["en"], "gender": "male", "type": "preset"},
        {"name": "no id"},
        {"id": "mine", "name": "My voice", "type": "custom"},
    ], "next_page_token": null});
    let (url, server) = serve(vec![(200, answer.to_string())]);
    let voices = client(&url).voices().unwrap();
    assert_eq!(voices.len(), 2);
    // The owner's own voice comes first.
    assert_eq!(voices[0].id, "mine");
    assert!(voices[0].custom);
    assert_eq!(voices[1].id, "a1");
    assert!(!voices[1].custom);
    assert_eq!(voices[1].languages, vec!["en"]);
    assert_eq!(voices[1].slug, "paul_neutral");
    let seen = server.join().unwrap();
    assert_eq!(seen[0].method, "GET");
    assert!(seen[0].path.starts_with("/v2/audio/voices?type=all"));
}

#[test]
fn keys_and_hosts_are_checked() {
    assert!(!valid_key("has space"));
    assert!(!valid_key(""));
    assert!(valid_key("abcDEF123"));
    assert!(
        Mistral::local(
            Secret::new("k".into()),
            "http://example.com",
            Duration::from_secs(1)
        )
        .is_err()
    );
    assert!(Mistral::new(Secret::new("bad key".into()), Duration::from_secs(1)).is_err());
}
