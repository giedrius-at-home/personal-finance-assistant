use std::cell::RefCell;

use super::*;

const TOKEN: &str = "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ";

#[test]
fn checks_tokens_and_chats() {
    assert!(valid_token(TOKEN));
    assert!(!valid_token("123456789"));
    assert!(!valid_token("abc:AAHdqTcvCH1vGWJxfSeofSAs0K5PALDsawQ"));
    assert!(!valid_token("123456789:short"));
    assert!(!valid_token(
        "123456789:AAHdqTcvCH1vGWJxfSeofSAs0K5PAL/sawQ"
    ));
    assert!(valid_chat("987654321"));
    assert!(valid_chat("-1001234567890"));
    assert!(!valid_chat(""));
    assert!(!valid_chat("-"));
    assert!(!valid_chat("12a"));
}

#[test]
fn finds_the_latest_private_chat() {
    let updates = json!([
        { "update_id": 1, "message": { "chat": { "id": 111, "type": "private" } } },
        { "update_id": 2, "message": { "chat": { "id": -500, "type": "group" } } },
        { "update_id": 3, "message": { "chat": { "id": 222, "type": "private", "first_name": "Giedrius" } } },
        { "update_id": 4, "message": { "chat": { "id": -600, "type": "supergroup" } } },
    ]);
    assert_eq!(
        owner_from_updates(&updates),
        Some(Owner {
            chat: "222".into(),
            first_name: Some("Giedrius".into())
        })
    );
    assert_eq!(owner_from_updates(&json!([])), None);
}

#[test]
fn caption_fits_and_is_escaped() {
    let text = brief_text("Bills & <you>", "Rent goes out. ", MAX_CAPTION);
    assert_eq!(text, "<b>Bills &amp; &lt;you&gt;</b>\n\nRent goes out. ");
    let long = "a & b ".repeat(400);
    let text = brief_text("Title", &long, MAX_CAPTION);
    assert!(text.chars().count() <= MAX_CAPTION);
    assert!(text.ends_with('…'));
    let tail = &text[text.rfind('&').unwrap()..];
    assert!(tail.starts_with("&amp;"), "no half entity: {tail}");
}

#[test]
fn multipart_has_fields_and_file() {
    let upload = Upload {
        fields: vec![("chat_id", "42".into())],
        file: ("voice", "brief.mp3", "audio/mpeg", b"ID3data".to_vec()),
    };
    let (body, kind) = upload.encode();
    let boundary = kind.strip_prefix("multipart/form-data; boundary=").unwrap();
    let text = String::from_utf8(body).unwrap();
    assert!(text.starts_with(&format!("--{boundary}\r\n")));
    assert!(text.contains("name=\"chat_id\"\r\n\r\n42\r\n"));
    assert!(text.contains(
        "name=\"voice\"; filename=\"brief.mp3\"\r\nContent-Type: audio/mpeg\r\n\r\nID3data\r\n"
    ));
    assert!(text.ends_with(&format!("--{boundary}--\r\n")));
}

#[test]
fn boundary_avoids_the_file_bytes() {
    let clash = b"xx--finance-brief-00000000yy".to_vec();
    let (_, kind) = Upload {
        fields: vec![],
        file: ("voice", "brief.mp3", "audio/mpeg", clash),
    }
    .encode();
    assert!(kind.ends_with("finance-brief-00000001"));
}

#[test]
fn reads_telegrams_refusal() {
    assert_eq!(result(json!({"ok": true, "result": 5})), Ok(json!(5)));
    assert_eq!(
        result(
            json!({"ok": false, "error_code": 403, "description": "Forbidden: bot was blocked by the user"})
        ),
        Err("Telegram refused: Forbidden: bot was blocked by the user".into())
    );
}

#[derive(Default)]
struct Stub {
    calls: RefCell<Vec<(String, Request)>>,
}

impl Api for Stub {
    fn call(&self, _token: &Secret, method: &str, request: Request) -> Result<Value, String> {
        self.calls.borrow_mut().push((method.into(), request));
        Ok(match method {
            "getMe" => json!({ "username": "gisel_money_bot" }),
            _ => json!(true),
        })
    }
}

#[test]
fn sends_voice_when_there_is_audio_and_text_otherwise() {
    let api = Stub::default();
    let token = Secret::new(TOKEN.into());
    let bot = Bot {
        api: &api,
        token: &token,
    };
    assert_eq!(bot.username().unwrap(), "@gisel_money_bot");
    bot.send_brief("42", "Hola", "Your money.", Some(b"mp3".to_vec()))
        .unwrap();
    bot.send_brief("42", "Hola", "Your money.", None).unwrap();
    bot.send_animation("42", "https://media.giphy.com/media/x/giphy.mp4")
        .unwrap();
    let calls = api.calls.borrow();
    assert_eq!(calls[1].0, "sendVoice");
    let Request::Upload(upload) = &calls[1].1 else {
        panic!("voice is an upload")
    };
    assert_eq!(upload.file.3, b"mp3");
    assert!(
        upload
            .fields
            .contains(&("caption", "<b>Hola</b>\n\nYour money.".into()))
    );
    assert_eq!(calls[2].0, "sendMessage");
    let Request::Json(body) = &calls[2].1 else {
        panic!("text is JSON")
    };
    assert_eq!(body["chat_id"], "42");
    assert_eq!(body["parse_mode"], "HTML");
    assert_eq!(calls[3].0, "sendAnimation");
    let Request::Json(body) = &calls[3].1 else {
        panic!("animation is JSON")
    };
    assert_eq!(
        body["animation"],
        "https://media.giphy.com/media/x/giphy.mp4"
    );
}
