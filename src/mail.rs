//! IMAP sync, SMTP sending and message parsing.

use imap::types::Flag;
use imap::{ClientBuilder, ConnectionMode, TlsKind};
use imap_proto::NameAttribute;
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{SmtpTransport, Transport};
use mail_parser::MessageParser;

use crate::store::{Account, Message, Security, Server, Store};
use crate::text;

/// How many of the newest messages per folder to download on the first sync.
const INITIAL_SYNC: u32 = 300;
/// Folder name used for sent messages that are not on the server yet.
const LOCAL_FOLDER: &str = "(local)";

type Session = imap::Session<imap::Connection>;
type Result<T> = std::result::Result<T, String>;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn connect_imap(server: &Server, password: &str) -> Result<Session> {
    let mode = match server.security {
        Security::Tls => ConnectionMode::Tls,
        Security::StartTls => ConnectionMode::StartTls,
        Security::None => ConnectionMode::Plaintext,
    };
    let client = ClientBuilder::new(server.host.as_str(), server.port)
        .mode(mode)
        .tls_kind(TlsKind::Native)
        .connect()
        .map_err(|e| format!("IMAP connection failed: {e}"))?;
    client
        .login(&server.username, password)
        .map_err(|(e, _)| format!("IMAP login failed: {e}"))
}

fn smtp_transport(server: &Server, password: &str) -> Result<SmtpTransport> {
    let builder = match server.security {
        Security::Tls => SmtpTransport::relay(&server.host).map_err(err)?,
        Security::StartTls => SmtpTransport::starttls_relay(&server.host).map_err(err)?,
        Security::None => SmtpTransport::builder_dangerous(&server.host),
    };
    Ok(builder
        .port(server.port)
        .credentials(Credentials::new(server.username.clone(), password.to_string()))
        .timeout(Some(std::time::Duration::from_secs(30)))
        .build())
}

/// Log in to IMAP and SMTP to check the settings.
pub fn test_account(account: &Account, password: &str) -> Result<()> {
    let mut session = connect_imap(&account.imap, password)?;
    let _ = session.logout();
    if !account.smtp.host.is_empty() {
        let ok = smtp_transport(&account.smtp, password)?
            .test_connection()
            .map_err(|e| format!("SMTP connection failed: {e}"))?;
        if !ok {
            return Err("SMTP server did not respond".into());
        }
    }
    Ok(())
}

fn find_sent_folder(session: &mut Session) -> Option<String> {
    let names = session.list(None, Some("*")).ok()?;
    if let Some(n) = names.iter().find(|n| n.attributes().contains(&NameAttribute::Sent)) {
        return Some(n.name().to_string());
    }
    let wanted = [
        "sent",
        "sent items",
        "sent messages",
        "sent mail",
        "gesendet",
        "odeslané",
    ];
    names
        .iter()
        .find(|n| {
            let leaf = n.name().rsplit(n.delimiter().unwrap_or("/")).next().unwrap_or("");
            wanted.contains(&leaf.to_lowercase().as_str())
        })
        .map(|n| n.name().to_string())
}

/// Turn a raw RFC 822 message into a cached message.
pub fn parse(raw: &[u8], folder: &str, uid: Option<u32>, seen: bool, sent_folder: bool, me: &str) -> Option<Message> {
    let msg = MessageParser::default().parse(raw)?;
    let mailbox = |m: mail_parser::Mailbox| {
        (
            m.address().unwrap_or("").to_lowercase(),
            m.name().unwrap_or("").to_string(),
        )
    };
    let from = msg.from().and_then(|a| a.first()).map(mailbox).unwrap_or_default();
    let recipients: Vec<(String, String)> = msg
        .to()
        .into_iter()
        .chain(msg.cc())
        .flat_map(|list| list.mailboxes().map(mailbox).collect::<Vec<_>>())
        .collect();
    let outgoing = sent_folder || from.0 == me;
    let (contact, contact_name) = if outgoing {
        recipients
            .first()
            .cloned()
            .unwrap_or_else(|| ("(no recipient)".into(), String::new()))
    } else {
        from
    };

    let html = msg
        .html_body()
        .any(|p| p.is_content_type("text", "html"))
        .then(|| msg.body_html(0).map(|h| h.into_owned()))
        .flatten();
    let body = match msg.text_body().any(|p| !p.is_content_type("text", "html")) {
        true => msg.body_text(0).map(|t| t.into_owned()).unwrap_or_default(),
        false => html.as_deref().map(text::html_to_text).unwrap_or_default(),
    };
    let body = body.replace("\r\n", "\n");

    let message_id = msg
        .message_id()
        .map(str::to_string)
        .unwrap_or_else(|| format!("mailer-no-id:{folder}:{}", uid.unwrap_or(0)));
    let mut references: Vec<String> = msg
        .references()
        .map(|r| r.iter().map(str::to_string).collect())
        .unwrap_or_default();
    if references.is_empty() {
        references = msg
            .in_reply_to()
            .map(|r| r.iter().map(str::to_string).collect())
            .unwrap_or_default();
    }

    Some(Message {
        message_id,
        folder: folder.to_string(),
        uid,
        outgoing,
        contact,
        contact_name,
        subject: msg.subject().unwrap_or("").to_string(),
        date: msg
            .date()
            .map(|d| d.to_timestamp())
            .unwrap_or_else(|| chrono::Utc::now().timestamp()),
        short: text::strip_quoted(&body),
        body,
        html,
        references,
        attachments: msg
            .attachments()
            .filter(|a| a.attachment_name().is_some())
            .map(|a| a.attachment_name().unwrap_or("attachment").to_string())
            .collect(),
        seen: outgoing || seen,
    })
}

fn sync_folder(session: &mut Session, store: &mut Store, path: &str, sent: bool, me: &str) -> Result<usize> {
    let mailbox = session.select(path).map_err(err)?;
    let validity = mailbox.uid_validity.unwrap_or(0);
    let mut last_uid = match store.folder_state(path) {
        Some((v, last)) if v == validity => last,
        Some(_) => {
            store.drop_folder(path).map_err(err)?;
            0
        }
        None => 0,
    };
    if mailbox.exists == 0 {
        store.set_folder_state(path, validity, last_uid).map_err(err)?;
        return Ok(0);
    }

    // Update read state of messages we already have.
    if let Some(min) = store.min_uid(path) {
        let fetches = session.uid_fetch(format!("{min}:*"), "(UID FLAGS)").map_err(err)?;
        let flags: Vec<(u32, bool)> = fetches
            .iter()
            .filter_map(|f| Some((f.uid?, f.flags().contains(&Flag::Seen))))
            .collect();
        store.set_seen_flags(path, &flags).map_err(err)?;
    }

    let query = "(UID FLAGS BODY.PEEK[])";
    let fetches = if last_uid == 0 {
        let start = mailbox.exists.saturating_sub(INITIAL_SYNC - 1).max(1);
        session.fetch(format!("{start}:*"), query)
    } else {
        // "n:*" always returns the newest message, even if its UID is below n.
        session.uid_fetch(format!("{}:*", last_uid + 1), query)
    }
    .map_err(err)?;

    let mut added = 0;
    for f in fetches.iter() {
        let (Some(uid), Some(raw)) = (f.uid, f.body()) else {
            continue;
        };
        if uid <= last_uid {
            continue;
        }
        if let Some(m) = parse(raw, path, Some(uid), f.flags().contains(&Flag::Seen), sent, me) {
            store.upsert(&m).map_err(err)?;
            added += 1;
        }
        last_uid = last_uid.max(uid);
    }
    store.set_folder_state(path, validity, last_uid).map_err(err)?;
    Ok(added)
}

/// Pull all new mail from INBOX and the Sent folder. Returns the number of new messages.
pub fn refresh(account: &Account, password: &str) -> Result<usize> {
    let mut store = Store::open().map_err(err)?;
    let mut session = connect_imap(&account.imap, password)?;
    let me = account.email.to_lowercase();
    let result = (|| {
        let mut added = sync_folder(&mut session, &mut store, "INBOX", false, &me)?;
        if let Some(sent) = find_sent_folder(&mut session) {
            added += sync_folder(&mut session, &mut store, &sent, true, &me)?;
        }
        Ok(added)
    })();
    let _ = session.logout();
    result
}

/// Set the \Seen flag on the server for all unread messages from a contact.
pub fn mark_read(account: &Account, password: &str, contact: &str) -> Result<()> {
    let store = Store::open().map_err(err)?;
    let unread = store.unread(contact).map_err(err)?;
    let mut by_folder: std::collections::BTreeMap<String, Vec<String>> = Default::default();
    for (folder, uid) in unread.into_iter().filter(|(f, _)| f != LOCAL_FOLDER) {
        by_folder.entry(folder).or_default().push(uid.to_string());
    }
    if !by_folder.is_empty() {
        let mut session = connect_imap(&account.imap, password)?;
        for (folder, uids) in &by_folder {
            session.select(folder).map_err(err)?;
            session.uid_store(uids.join(","), "+FLAGS (\\Seen)").map_err(err)?;
        }
        let _ = session.logout();
    }
    store.mark_seen(contact).map_err(err)
}

fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Send a message to a contact, threaded onto the last message of the conversation.
pub fn send(account: &Account, password: &str, to: &str, subject: &str, body: &str, signature: &str) -> Result<()> {
    if account.smtp.host.is_empty() {
        return Err("No SMTP server configured".into());
    }
    let store = Store::open().map_err(err)?;
    let last = store.thread(to).map_err(err)?.pop();

    let subject = match (subject.trim(), &last) {
        ("", Some(m)) if m.subject.to_lowercase().starts_with("re:") => m.subject.clone(),
        ("", Some(m)) => format!("Re: {}", m.subject),
        ("", None) => "(no subject)".into(),
        (s, _) => s.to_string(),
    };

    let mut html = format!(
        "<div style=\"font-family:sans-serif;font-size:14px\">{}</div>",
        escape_html(body).replace('\n', "<br>")
    );
    let mut plain = body.to_string();
    if !signature.trim().is_empty() {
        html.push_str(&format!("<br><div class=\"signature\">{signature}</div>"));
        plain.push_str(&format!("\n\n-- \n{}", text::html_to_text(signature)));
    }

    let from = Mailbox::new(
        (!account.name.is_empty()).then(|| account.name.clone()),
        account.email.parse().map_err(err)?,
    );
    let mut builder = lettre::Message::builder()
        .from(from)
        .to(to.parse().map_err(|e| format!("Invalid recipient: {e}"))?)
        .subject(subject)
        .message_id(None);
    if let Some(m) = last.as_ref().filter(|m| !m.message_id.starts_with("mailer-no-id:")) {
        let mut refs: Vec<String> = m.references.iter().map(|r| format!("<{r}>")).collect();
        refs.push(format!("<{}>", m.message_id));
        builder = builder
            .in_reply_to(format!("<{}>", m.message_id))
            .references(refs.join(" "));
    }
    let message = builder
        .multipart(MultiPart::alternative_plain_html(plain, html))
        .map_err(err)?;

    smtp_transport(&account.smtp, password)?
        .send(&message)
        .map_err(|e| format!("Sending failed: {e}"))?;
    let raw = message.formatted();

    // Keep a copy in the Sent folder. Gmail does this on its own.
    let host = account.smtp.host.to_lowercase();
    if !host.ends_with("gmail.com")
        && !host.ends_with("googlemail.com")
        && let Ok(mut session) = connect_imap(&account.imap, password)
    {
        if let Some(sent) = find_sent_folder(&mut session) {
            let _ = session.append(&sent, &raw).flag(Flag::Seen).finish();
        }
        let _ = session.logout();
    }

    if let Some(m) = parse(&raw, LOCAL_FOLDER, None, true, true, &account.email.to_lowercase()) {
        store.upsert(&m).map_err(err)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs against a local test server, see `MAILER_TEST_IMAP_PORT`.
    #[test]
    #[ignore]
    fn sync_and_send_against_test_server() {
        let port = |var: &str| std::env::var(var).unwrap().parse().unwrap();
        let account = Account {
            email: "me@test.local".into(),
            name: "Me".into(),
            imap: Server {
                host: "127.0.0.1".into(),
                port: port("MAILER_TEST_IMAP_PORT"),
                security: Security::None,
                username: "me".into(),
            },
            smtp: Server {
                host: "127.0.0.1".into(),
                port: port("MAILER_TEST_SMTP_PORT"),
                security: Security::None,
                username: "me".into(),
            },
        };
        test_account(&account, "pw").unwrap();
        let added = refresh(&account, "pw").unwrap();
        assert!(added >= 4, "added {added}");
        assert_eq!(refresh(&account, "pw").unwrap(), 0);

        let store = Store::open().unwrap();
        let convs = store.conversations().unwrap();
        let alice = convs.iter().find(|c| c.contact == "alice@ex.com").unwrap();
        assert_eq!(alice.name, "Alice");
        assert_eq!(alice.unread, 2);

        mark_read(&account, "pw", "alice@ex.com").unwrap();
        send(&account, "pw", "alice@ex.com", "", "Running late!", "<b>Me</b>").unwrap();
        assert_eq!(refresh(&account, "pw").unwrap(), 1);

        let thread = store.thread("alice@ex.com").unwrap();
        let last = thread.last().unwrap();
        assert_eq!(last.short, "Running late!");
        assert_eq!(last.subject, "Re: Lunch?");
        assert!(last.html.as_deref().unwrap().contains("<b>Me</b>"));
        assert_eq!(thread.iter().filter(|m| m.short == "Running late!").count(), 1);
        assert!(store.conversations().unwrap().iter().all(|c| c.unread == 0));
    }
}
