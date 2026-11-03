//! Plain text helpers.

pub fn html_to_text(html: &str) -> String {
    html2text::from_read_with_decorator(html.as_bytes(), 10_000, html2text::render::TrivialDecorator::new())
        .unwrap_or_default()
        .trim()
        .to_string()
}

/// A full HTML document for showing mail or signature HTML in a browser.
/// The content security policy blocks scripts, and remote content unless allowed.
pub fn html_document(html: &str, allow_remote: bool) -> String {
    let img = if allow_remote {
        "img-src * data:"
    } else {
        "img-src data:"
    };
    format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; {img}; \
         style-src 'unsafe-inline'; font-src data:\">\
         <style>body{{font-family:sans-serif;font-size:14px;margin:16px}}</style>{html}"
    )
}

const QUOTE_MARKERS: &[fn(&str) -> bool] = &[
    // "On Mon, 2 Nov 2026, Alice wrote:"
    |l| l.starts_with("On ") && l.ends_with("wrote:"),
    // German clients: "Am ... schrieb ...:"
    |l| l.starts_with("Am ") && l.contains(" schrieb ") && l.ends_with(':'),
    |l| {
        let l = l.to_lowercase();
        l.starts_with("--") && (l.contains("original message") || l.contains("forwarded message"))
    },
    |l| l.starts_with("From: "),
    |l| l.len() >= 10 && l.chars().all(|c| c == '_'),
    // Signature delimiter "-- "
    |l| l == "--",
];

/// Cut off quoted history and the signature so a reply reads like a chat message.
pub fn strip_quoted(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut cut = lines.len();
    for (i, raw) in lines.iter().enumerate() {
        let l = raw.trim();
        if QUOTE_MARKERS.iter().any(|m| m(l)) {
            cut = i;
            break;
        }
        // A trailing block of "> " lines.
        if l.starts_with('>')
            && lines[i..]
                .iter()
                .all(|x| x.trim().is_empty() || x.trim().starts_with('>'))
        {
            cut = i;
            break;
        }
    }
    lines[..cut].join("\n").trim_end().to_string()
}

/// Subject without "Re:", "Fwd:" and similar prefixes, for grouping topics.
pub fn base_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_lowercase();
        let Some(prefix) = ["re:", "fwd:", "fw:", "aw:", "wg:"]
            .iter()
            .find(|p| lower.starts_with(*p))
        else {
            break;
        };
        s = s[prefix.len()..].trim_start();
    }
    s.to_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_reply_history() {
        let t = "Great, see you!\n\nOn Mon, Nov 2, 2026 Bob wrote:\n> sure";
        assert_eq!(strip_quoted(t), "Great, see you!");
    }

    #[test]
    fn strips_signature() {
        assert_eq!(strip_quoted("Hello\n\n-- \nJane\nexample.com"), "Hello");
    }

    #[test]
    fn keeps_inline_quotes() {
        let t = "> question?\nanswer";
        assert_eq!(strip_quoted(t), t);
    }

    #[test]
    fn base_subject_drops_prefixes() {
        assert_eq!(base_subject("Re: FWD: re: Lunch"), "lunch");
    }
}
