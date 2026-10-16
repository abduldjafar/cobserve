//! Telegram's Bot API (`telegram.rs` is the bot): updates read by long polling, and what the bot
//! says sent one at a time, in order. The token is in every URL, so no URL is ever put into an
//! error: reqwest's errors go out `without_url`.

use crate::telegram::{BotEvent, Incoming, Out};
use std::time::Duration;
use tokio::sync::mpsc;

const API: &str = "https://api.telegram.org";
/// How long Telegram holds a poll open when there is nothing to say.
const HOLD_S: u64 = 50;

/// Starts both halves; what the bot says goes into the sender it returns.
pub fn spawn(token: String, tx: mpsc::UnboundedSender<BotEvent>) -> mpsc::UnboundedSender<Out> {
    let client = reqwest::Client::builder().build().unwrap_or_default();
    let (out_tx, out_rx) = mpsc::unbounded_channel::<Out>();
    tokio::spawn(read(client.clone(), token.clone(), tx.clone()));
    tokio::spawn(send(client, token, out_rx, tx));
    out_tx
}

async fn read(client: reqwest::Client, token: String, tx: mpsc::UnboundedSender<BotEvent>) {
    let mut offset: i64 = 0;
    let mut last_trouble = String::new();
    loop {
        let url = format!("{API}/bot{token}/getUpdates");
        let body = serde_json::json!({ "offset": offset, "timeout": HOLD_S, "allowed_updates": ["message", "callback_query"] });
        let answer = client.post(&url).timeout(Duration::from_secs(HOLD_S + 15)).json(&body).send().await;
        let trouble = match answer {
            Ok(response) => {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                match status.as_u16() {
                    200 => match parse_updates(&text) {
                        Ok((updates, next)) => {
                            offset = offset.max(next);
                            for update in updates {
                                if tx.send(BotEvent::Incoming(update)).is_err() {
                                    return;
                                }
                            }
                            last_trouble.clear();
                            continue;
                        }
                        Err(why) => why,
                    },
                    401 | 404 => {
                        let _ = tx.send(BotEvent::Trouble("the bot token was refused — check TELEGRAM_BOT_TOKEN or TELEGRAM_TOKEN_FILE".to_string()));
                        return;
                    }
                    409 => "another program is reading this bot's messages (a second cobserve?) — this one waits".to_string(),
                    code => format!("HTTP {code} from Telegram"),
                }
            }
            Err(e) => super::transport(e, Duration::from_secs(HOLD_S + 15)),
        };
        if trouble != last_trouble {
            let _ = tx.send(BotEvent::Trouble(trouble.clone()));
            last_trouble = trouble;
        }
        tokio::time::sleep(Duration::from_secs(if last_trouble.contains("another program") { 30 } else { 5 })).await;
        if tx.is_closed() {
            return;
        }
    }
}

async fn send(client: reqwest::Client, token: String, mut rx: mpsc::UnboundedReceiver<Out>, tx: mpsc::UnboundedSender<BotEvent>) {
    while let Some(out) = rx.recv().await {
        let (method, mut body) = request(&out);
        let url = format!("{API}/bot{token}/{method}");
        let mut trouble = post(&client, &url, method, &body).await;
        // HTML Telegram will not parse goes again as the text it is, so nothing is lost to a tag.
        if trouble.as_deref().is_some_and(|t| t.contains("can't parse entities"))
            && let Some(text) = body.get("text").and_then(|t| t.as_str()).map(crate::telegram::html::plain_of)
        {
            body["text"] = serde_json::Value::String(text);
            if let Some(object) = body.as_object_mut() {
                object.remove("parse_mode");
            }
            trouble = post(&client, &url, method, &body).await;
        }
        if let Some(trouble) = trouble {
            let _ = tx.send(BotEvent::Trouble(trouble));
        }
    }
}

/// One call; what went wrong with it, if anything.
async fn post(client: &reqwest::Client, url: &str, method: &str, body: &serde_json::Value) -> Option<String> {
    match client.post(url).timeout(Duration::from_secs(20)).json(body).send().await {
        Ok(response) if response.status().is_success() => None,
        // An edit that changes nothing is refused; that is not trouble.
        Ok(response) => {
            let code = response.status().as_u16();
            let text = response.text().await.unwrap_or_default();
            (!text.contains("message is not modified")).then(|| format!("{method}: HTTP {code} {}", description(&text)))
        }
        Err(e) => Some(format!("{method}: {}", super::transport(e, Duration::from_secs(20)))),
    }
}

/// The method and the body that say `out`. What the bot says is Telegram's HTML, every piece
/// of it built with `telegram::esc`, so a `<` in a node's name or an answer cannot break it.
pub fn request(out: &Out) -> (&'static str, serde_json::Value) {
    match out {
        Out::Send { chat, text, buttons } => {
            let mut body = serde_json::json!({ "chat_id": chat, "text": text, "parse_mode": "HTML", "link_preview_options": { "is_disabled": true } });
            if !buttons.is_empty() {
                body["reply_markup"] = keyboard(buttons);
            }
            ("sendMessage", body)
        }
        Out::Edit { chat, message, text, buttons } => {
            let mut body = serde_json::json!({ "chat_id": chat, "message_id": message, "text": text, "parse_mode": "HTML", "link_preview_options": { "is_disabled": true } });
            if !buttons.is_empty() {
                body["reply_markup"] = keyboard(buttons);
            }
            ("editMessageText", body)
        }
        Out::Answer { callback, text } => ("answerCallbackQuery", serde_json::json!({ "callback_query_id": callback, "text": text })),
        Out::Typing { chat } => ("sendChatAction", serde_json::json!({ "chat_id": chat, "action": "typing" })),
    }
}

/// The buttons, two to a row — or in the rows `telegram::ROW` breaks them into.
fn keyboard(buttons: &[(String, String)]) -> serde_json::Value {
    let button = |(label, data): &(String, String)| serde_json::json!({ "text": label, "callback_data": data });
    let rows: Vec<Vec<serde_json::Value>> = if buttons.iter().any(|b| b.0.is_empty()) {
        buttons.split(|b| b.0.is_empty()).filter(|row| !row.is_empty()).map(|row| row.iter().map(button).collect()).collect()
    } else {
        buttons.chunks(2).map(|row| row.iter().map(button).collect()).collect()
    };
    serde_json::json!({ "inline_keyboard": rows })
}

fn description(text: &str) -> String {
    serde_json::from_str::<serde_json::Value>(text)
        .ok()
        .and_then(|v| v.get("description").and_then(|d| d.as_str()).map(str::to_string))
        .unwrap_or_default()
        .chars()
        .take(200)
        .collect()
}

/// `getUpdates`' answer: the messages and button presses in it, and the offset after them.
pub fn parse_updates(text: &str) -> Result<(Vec<Incoming>, i64), String> {
    let value: serde_json::Value = serde_json::from_str(text).map_err(|e| format!("Telegram's answer did not parse: {e}"))?;
    if value.get("ok").and_then(|o| o.as_bool()) != Some(true) {
        return Err(format!("Telegram said no: {}", description(text)));
    }
    let mut updates = Vec::new();
    let mut next = 0;
    for update in value.get("result").and_then(|r| r.as_array()).into_iter().flatten() {
        if let Some(id) = update.get("update_id").and_then(|i| i.as_i64()) {
            next = next.max(id + 1);
        }
        if let Some(message) = update.get("message") {
            let chat = message.pointer("/chat/id").and_then(|c| c.as_i64());
            let text = message.get("text").and_then(|t| t.as_str());
            let at = message.get("date").and_then(|d| d.as_i64()).unwrap_or(0);
            if let (Some(chat), Some(text)) = (chat, text) {
                updates.push(Incoming::Message { chat, text: text.to_string(), at });
            }
        } else if let Some(query) = update.get("callback_query") {
            let chat = query.pointer("/message/chat/id").and_then(|c| c.as_i64());
            let message = query.pointer("/message/message_id").and_then(|m| m.as_i64());
            let callback = query.get("id").and_then(|i| i.as_str());
            let data = query.get("data").and_then(|d| d.as_str()).unwrap_or_default();
            if let (Some(chat), Some(message), Some(callback)) = (chat, message, callback) {
                updates.push(Incoming::Button { chat, message, callback: callback.to_string(), data: data.to_string() });
            }
        }
    }
    Ok((updates, next))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_and_presses_come_out_of_an_update() {
        let text = r#"{"ok":true,"result":[
            {"update_id":10,"message":{"message_id":5,"date":1791100000,"chat":{"id":77,"type":"private"},"text":"node mana yang penuh?"}},
            {"update_id":11,"message":{"message_id":6,"date":1791100001,"chat":{"id":77},"sticker":{}}},
            {"update_id":12,"callback_query":{"id":"cb1","data":"y:3","message":{"message_id":9,"chat":{"id":77}}}}
        ]}"#;
        let (updates, next) = parse_updates(text).unwrap();
        assert_eq!(next, 13);
        assert_eq!(
            updates,
            [
                Incoming::Message { chat: 77, text: "node mana yang penuh?".into(), at: 1_791_100_000 },
                Incoming::Button { chat: 77, message: 9, callback: "cb1".into(), data: "y:3".into() },
            ]
        );
        assert!(parse_updates(r#"{"ok":false,"description":"Unauthorized"}"#).unwrap_err().contains("Unauthorized"));
    }

    #[test]
    fn a_card_is_sent_with_its_buttons_as_html() {
        let (method, body) = request(&Out::Send { chat: 1, text: "Kill this query?".into(), buttons: vec![("🛑 Kill it".into(), "y:2".into()), ("Leave it".into(), "n:2".into())] });
        assert_eq!(method, "sendMessage");
        assert_eq!(body["reply_markup"]["inline_keyboard"][0][1]["callback_data"], "n:2");
        assert_eq!(body["parse_mode"], "HTML");
        let rows = keyboard(&[("a".into(), "1".into()), ("b".into(), "2".into()), ("c".into(), "3".into()), (String::new(), String::new()), ("d".into(), "4".into())]);
        assert_eq!(rows["inline_keyboard"].as_array().unwrap().len(), 2);
        assert_eq!(rows["inline_keyboard"][0].as_array().unwrap().len(), 3);
    }
}
