//! A stand-in Mistral on 127.0.0.1 for tests: answers each request with the
//! next canned JSON, and hands back what it was sent.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::thread::JoinHandle;

/// One request as the fake saw it.
#[derive(Clone, Debug)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub authorization: String,
    pub body: String,
}

/// Serve `answers` (status, JSON body) in order, one connection each.
pub fn serve(answers: Vec<(u16, String)>) -> (String, JoinHandle<Vec<Seen>>) {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
    let url = format!("http://{}", listener.local_addr().expect("addr"));
    let handle = std::thread::spawn(move || {
        let mut seen = Vec::new();
        for (status, answer) in answers {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(stream.try_clone().expect("clone"));
            let mut first = String::new();
            reader.read_line(&mut first).expect("request line");
            let mut parts = first.split_whitespace();
            let method = parts.next().unwrap_or_default().to_owned();
            let path = parts.next().unwrap_or_default().to_owned();
            let (mut length, mut authorization) = (0, String::new());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).expect("header");
                let lower = line.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    length = v.trim().parse().expect("length");
                }
                if lower.starts_with("authorization:") {
                    authorization = line["authorization:".len()..].trim().to_owned();
                }
                if line == "\r\n" {
                    break;
                }
            }
            let mut body = vec![0; length];
            std::io::Read::read_exact(&mut reader, &mut body).expect("body");
            write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\
                 Connection: close\r\n\r\n{answer}",
                answer.len()
            )
            .expect("write");
            seen.push(Seen {
                method,
                path,
                authorization,
                body: String::from_utf8(body).expect("utf-8"),
            });
        }
        seen
    });
    (url, handle)
}

/// A chat completion whose message is `content`.
pub fn chat(content: &str) -> String {
    serde_json::json!({"choices": [{"message": {"role": "assistant", "content": content}}]})
        .to_string()
}
