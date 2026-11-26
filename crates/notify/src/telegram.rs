//! The daily brief on Telegram: Gisel and Rafael each have their own bot
//! (made with @BotFather), and the brief comes from the one who speaks it,
//! as a voice message (`sendVoice` takes the MP3 Voxtral makes) or as text
//! when there is no audio. Blocking, like the mailer: callers run it on
//! their own thread. The token is part of every Bot API URL, so no error
//! text here ever carries a URL.

use std::time::Duration;

use finance_secrets::Secret;
use serde_json::{Value, json};

/// Telegram's limit on a voice message's caption.
pub const MAX_CAPTION: usize = 1024;
/// Telegram's limit on a text message.
pub const MAX_TEXT: usize = 4096;
/// A Bot API reply is a few kilobytes at most.
const MAX_REPLY: usize = 256 * 1024;

/// Whether `token` looks like a bot token: the bot's number, a colon and
/// the secret part (`123456789:AAH…`).
pub fn valid_token(token: &str) -> bool {
    let Some((id, secret)) = token.split_once(':') else {
        return false;
    };
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|b| b.is_ascii_digit())
        && (30..=100).contains(&secret.len())
        && secret
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// Whether `chat` is a chat id: digits, with a minus for groups.
pub fn valid_chat(chat: &str) -> bool {
    let digits = chat.strip_prefix('-').unwrap_or(chat);
    !digits.is_empty() && digits.len() <= 20 && digits.bytes().all(|b| b.is_ascii_digit())
}

/// At most `max` characters of `text`, ending in `…` when cut.
pub fn clip(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// What goes under the voice message, or is the message without audio:
/// the title in bold, then the words.
pub fn brief_text(title: &str, words: &str, max: usize) -> String {
    let head = format!("<b>{}</b>\n\n", escape(title));
    let room = max.saturating_sub(head.chars().count());
    // Clip before escaping would let `&amp;` grow past the limit; clip the
    // escaped text at a character that does not split an entity.
    let body = escape(words);
    let mut cut = clip(&body, room);
    if let Some(amp) = cut.rfind('&')
        && !cut[amp..].contains(';')
    {
        cut.truncate(amp);
        cut.push('…');
    }
    format!("{head}{cut}")
}

/// Text made safe for Telegram's HTML parse mode.
pub fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The owner, as Telegram knows them: their private chat with the bot and
/// their first name there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Owner {
    pub chat: String,
    pub first_name: Option<String>,
}

/// The private chat that last wrote to the bot, from a `getUpdates` reply:
/// how the owner's chat (and name) is found after they send the bot "hi".
pub fn owner_from_updates(reply: &Value) -> Option<Owner> {
    let chat = reply
        .as_array()?
        .iter()
        .rev()
        .filter_map(|u| u.get("message").or_else(|| u.get("my_chat_member")))
        .filter_map(|m| m.get("chat"))
        .find(|c| c.get("type").and_then(Value::as_str) == Some("private"))?;
    Some(Owner {
        chat: chat.get("id").and_then(Value::as_i64)?.to_string(),
        first_name: chat
            .get("first_name")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_owned),
    })
}

/// A file upload with its text fields, as `multipart/form-data`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub fields: Vec<(&'static str, String)>,
    /// The form field, file name, content type and bytes.
    pub file: (&'static str, &'static str, &'static str, Vec<u8>),
}

impl Upload {
    /// The request body and its `Content-Type`, with a boundary that does
    /// not occur in any part.
    pub fn encode(&self) -> (Vec<u8>, String) {
        let mut n = 0u32;
        let boundary = loop {
            let b = format!("finance-brief-{n:08x}");
            let clash = self.file.3.windows(b.len()).any(|w| w == b.as_bytes())
                || self.fields.iter().any(|(_, v)| v.contains(&b));
            if !clash {
                break b;
            }
            n += 1;
        };
        let mut body = Vec::with_capacity(self.file.3.len() + 1024);
        for (name, value) in &self.fields {
            body.extend_from_slice(
                format!(
                    "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
                )
                .as_bytes(),
            );
        }
        let (name, file, kind, bytes) = &self.file;
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"; filename=\"{file}\"\r\nContent-Type: {kind}\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (body, format!("multipart/form-data; boundary={boundary}"))
    }
}

/// One Bot API request.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Json(Value),
    Upload(Upload),
}

/// Calls a Bot API method and gives back its `result`: HTTPS in the app, a
/// stub in tests.
pub trait Api {
    fn call(&self, token: &Secret, method: &str, request: Request) -> Result<Value, String>;
}

/// `https://api.telegram.org`.
#[derive(Debug)]
pub struct Https(reqwest::blocking::Client);

impl Https {
    pub fn new() -> Result<Self, String> {
        reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(60))
            .build()
            .map(Self)
            .map_err(|e| format!("Telegram client: {}", e.without_url()))
    }
}

impl Api for Https {
    fn call(&self, token: &Secret, method: &str, request: Request) -> Result<Value, String> {
        let url = format!("https://api.telegram.org/bot{}/{method}", token.expose());
        let builder = self.0.post(url);
        let builder = match request {
            Request::Json(body) => builder.json(&body),
            Request::Upload(upload) => {
                let (body, kind) = upload.encode();
                builder
                    .header(reqwest::header::CONTENT_TYPE, kind)
                    .body(body)
            }
        };
        let response = builder
            .send()
            .map_err(|e| format!("Telegram could not be reached: {}", e.without_url()))?;
        let mut bytes = Vec::new();
        use std::io::Read;
        response
            .take(MAX_REPLY as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "Telegram's reply could not be read.".to_owned())?;
        if bytes.len() > MAX_REPLY {
            return Err("Telegram's reply was too long.".into());
        }
        let reply: Value = serde_json::from_slice(&bytes)
            .map_err(|_| "Telegram's reply was not JSON.".to_owned())?;
        result(reply)
    }
}

/// The `result` of a Bot API reply, or Telegram's own description of why
/// it refused.
pub fn result(reply: Value) -> Result<Value, String> {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
    }
    let why = reply
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("no reason given");
    Err(format!("Telegram refused: {}", clip(why, 200)))
}

/// One persona's bot.
#[derive(Debug)]
pub struct Bot<'a, A> {
    pub api: &'a A,
    pub token: &'a Secret,
}

impl<A: Api> Bot<'_, A> {
    /// The bot's `@username`: checks the token works.
    pub fn username(&self) -> Result<String, String> {
        let me = self
            .api
            .call(self.token, "getMe", Request::Json(json!({})))?;
        me.get("username")
            .and_then(Value::as_str)
            .map(|u| format!("@{u}"))
            .ok_or_else(|| "Telegram did not name the bot.".to_owned())
    }

    /// The owner's chat and name, once they have written to the bot.
    pub fn find_owner(&self) -> Result<Option<Owner>, String> {
        let updates = self.api.call(
            self.token,
            "getUpdates",
            Request::Json(
                json!({ "limit": 100, "allowed_updates": ["message", "my_chat_member"] }),
            ),
        )?;
        Ok(owner_from_updates(&updates))
    }

    pub fn send_text(&self, chat: &str, html: &str) -> Result<(), String> {
        self.api.call(
            self.token,
            "sendMessage",
            Request::Json(json!({
                "chat_id": chat,
                "text": html,
                "parse_mode": "HTML",
                "link_preview_options": { "is_disabled": true },
            })),
        )?;
        Ok(())
    }

    /// A GIF (or its MP4) by URL; Telegram fetches it.
    pub fn send_animation(&self, chat: &str, url: &str) -> Result<(), String> {
        self.api.call(
            self.token,
            "sendAnimation",
            Request::Json(json!({ "chat_id": chat, "animation": url })),
        )?;
        Ok(())
    }

    /// The MP3 as a voice message, with `caption` (HTML) under it.
    pub fn send_voice(&self, chat: &str, mp3: Vec<u8>, caption: &str) -> Result<(), String> {
        self.api.call(
            self.token,
            "sendVoice",
            Request::Upload(Upload {
                fields: vec![
                    ("chat_id", chat.to_owned()),
                    ("caption", caption.to_owned()),
                    ("parse_mode", "HTML".to_owned()),
                ],
                file: ("voice", "brief.mp3", "audio/mpeg", mp3),
            }),
        )?;
        Ok(())
    }

    /// The day's brief: spoken when there is audio, else as text.
    pub fn send_brief(
        &self,
        chat: &str,
        title: &str,
        words: &str,
        audio: Option<Vec<u8>>,
    ) -> Result<(), String> {
        match audio {
            Some(mp3) => self.send_voice(chat, mp3, &brief_text(title, words, MAX_CAPTION)),
            None => self.send_text(chat, &brief_text(title, words, MAX_TEXT)),
        }
    }
}

#[cfg(test)]
#[path = "telegram_tests.rs"]
mod tests;
