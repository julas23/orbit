//! What keeps a link from carrying you along: tracking parameters taken
//! off (`utm_*`, `fbclid`, …), redirect wrappers unwrapped (Google's
//! `/url?q=`, Facebook's `l.php?u=`), and the "same site" test that decides
//! whether a new window stays in the service or goes to the external
//! browser. The unread count read from a page title lives here too: plain
//! text work, kept apart from the widgets so it can be tested.

/// Query parameters that only exist to follow you.
const TRACKING: &[&str] = &[
    "fbclid", "gclid", "gclsrc", "dclid", "gbraid", "wbraid", "msclkid", "mc_cid", "mc_eid",
    "igshid", "igsh", "yclid", "twclid", "ttclid", "li_fat_id", "_hsenc", "_hsmi", "mkt_tok",
    "oly_anon_id", "oly_enc_id", "vero_id", "rb_clickid", "s_cid", "ref_src", "si",
];

/// Second-level names under which sites register ("com.br", "co.uk").
const SECOND_LEVEL: &[&str] = &["com", "net", "org", "gov", "edu", "co", "ac", "blog", "app"];

/// `url` without tracking parameters, and unwrapped when it is only a
/// redirect to another address.
pub fn clean(url: &str) -> String {
    let url = unwrap_redirect(url).unwrap_or_else(|| url.to_owned());
    let Some((base, rest)) = url.split_once('?') else { return url };
    let (query, fragment) = match rest.split_once('#') {
        Some((q, f)) => (q, Some(f)),
        None => (rest, None),
    };
    // "si" only tracks on YouTube and Spotify; elsewhere it may mean anything.
    let host = host(base).unwrap_or_default();
    let si_tracks = ["youtube.com", "youtu.be", "spotify.com"].iter().any(|h| same_site(&host, h));
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty())
        .filter(|p| {
            let key = p.split('=').next().unwrap_or_default().to_ascii_lowercase();
            let tracking = key.starts_with("utm_") || (TRACKING.contains(&key.as_str()) && (key != "si" || si_tracks));
            !tracking
        })
        .collect();
    let mut out = base.to_owned();
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(f) = fragment {
        out.push('#');
        out.push_str(f);
    }
    out
}

/// The address a redirect wrapper leads to.
fn unwrap_redirect(url: &str) -> Option<String> {
    let host = host(url)?;
    let (path, query) = url.split_once("://")?.1.split_once('/').map(|(_, p)| p.split_once('?').unwrap_or((p, "")))?;
    let param = |name: &str| {
        query
            .split('&')
            .find_map(|p| p.strip_prefix(name).and_then(|v| v.strip_prefix('=')))
            .map(percent_decode)
            .filter(|v| v.starts_with("http://") || v.starts_with("https://"))
    };
    if (host.starts_with("www.google.") || host.starts_with("google.")) && path == "url" {
        return param("q").or_else(|| param("url"));
    }
    if (host == "l.facebook.com" || host == "lm.facebook.com" || host == "l.instagram.com") && path == "l.php" {
        return param("u");
    }
    if host == "www.youtube.com" && path == "redirect" {
        return param("q");
    }
    None
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = |b: u8| (b as char).to_digit(16).map(|d| d as u8);
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => match (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                (Some(h), Some(l)) => {
                    out.push(h << 4 | l);
                    i += 3;
                    continue;
                }
                _ => out.push(b'%'),
            },
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The host of `url`, lowercase, without port or credentials.
pub fn host(url: &str) -> Option<String> {
    let rest = url.split_once("://")?.1;
    let authority = rest.split(['/', '?', '#']).next()?;
    let authority = authority.rsplit('@').next()?;
    let host = authority.split(':').next()?;
    (!host.is_empty()).then(|| host.to_ascii_lowercase())
}

/// The registered site of a host: "mail.google.com" → "google.com",
/// "www.zapimoveis.com.br" → "zapimoveis.com.br".
pub fn site(host: &str) -> String {
    let labels: Vec<&str> = host.trim_end_matches('.').split('.').collect();
    let n = labels.len();
    if n <= 2 {
        return host.to_owned();
    }
    let take = if labels[n - 1].len() == 2 && SECOND_LEVEL.contains(&labels[n - 2]) { 3 } else { 2 };
    labels[n - take.min(n)..].join(".")
}

/// Whether two hosts belong to the same registered site.
pub fn same_site(a: &str, b: &str) -> bool {
    site(a) == site(b)
}

/// Where sites send you to log in: these open inside the service, so the
/// login lands in its storage.
const LOGIN_HOSTS: &[&str] = &[
    "accounts.google.com",
    "login.microsoftonline.com",
    "login.live.com",
    "login.microsoft.com",
    "appleid.apple.com",
    "github.com",
];

/// Whether a new window for `url`, opened by a service at `service_url`,
/// stays inside it: the same site, one of its `domains`, a login page, or
/// a blank window a script will fill (logins do this).
pub fn stays_inside(url: &str, service_url: &str, domains: &[String]) -> bool {
    if url.is_empty() || url == "about:blank" {
        return true;
    }
    let (Some(h), Some(service)) = (host(url), host(service_url)) else { return false };
    same_site(&h, &service) || LOGIN_HOSTS.contains(&h.as_str()) || domains.iter().any(|d| same_site(&h, d))
}

/// The unread count in a page title — "(3) WhatsApp", "Caixa de entrada
/// (12) - Gmail", "[5] Telegram": the first number in brackets.
pub fn unread(title: &str) -> Option<u32> {
    let bytes = title.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        let close = match b {
            b'(' => b')',
            b'[' => b']',
            _ => continue,
        };
        let digits: String = title[i + 1..].chars().take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '+').collect();
        let after = i + 1 + digits.len();
        if !digits.is_empty() && bytes.get(after) == Some(&close) {
            let n: String = digits.chars().filter(char::is_ascii_digit).collect();
            if let Ok(n) = n.parse() {
                return Some(n);
            }
        }
    }
    None
}

/// What was typed in the address bar, as an address: as it is when it
/// has a scheme; with https:// when it looks like a host ("site.com",
/// "localhost:8080", "10.0.0.1/x"); otherwise a search in `search` (a URL
/// the query is appended to).
pub fn typed(text: &str, search: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        return String::new();
    }
    let lower = text.to_ascii_lowercase();
    if ["http://", "https://", "file://", "about:"].iter().any(|s| lower.starts_with(s)) {
        return clean(text);
    }
    let host = text.split(['/', '?', '#']).next().unwrap_or_default();
    let host_name = host.split(':').next().unwrap_or_default();
    let looks_like_host = !text.contains(char::is_whitespace)
        && !host_name.is_empty()
        && (host_name == "localhost"
            || (host_name.contains('.') && !host_name.starts_with('.') && !host_name.ends_with('.')))
        && host_name.chars().all(|c| c.is_alphanumeric() || c == '.' || c == '-');
    if looks_like_host {
        return clean(&format!("https://{text}"));
    }
    let query: String = text
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            b' ' => "+".into(),
            b => format!("%{b:02X}"),
        })
        .collect();
    format!("{search}{query}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_tracking() {
        assert_eq!(clean("https://a.com/p?utm_source=x&id=3&fbclid=abc#top"), "https://a.com/p?id=3#top");
        assert_eq!(clean("https://a.com/p?utm_medium=y"), "https://a.com/p");
        assert_eq!(clean("https://a.com/p?si=keep"), "https://a.com/p?si=keep");
        assert_eq!(clean("https://youtu.be/x?si=abc"), "https://youtu.be/x");
        assert_eq!(clean("https://a.com/"), "https://a.com/");
    }

    #[test]
    fn unwraps_redirects() {
        assert_eq!(
            clean("https://www.google.com/url?q=https%3A%2F%2Fexample.org%2Fa%3Futm_source%3Dm&sa=D"),
            "https://example.org/a"
        );
        assert_eq!(clean("https://l.facebook.com/l.php?u=https%3A%2F%2Fb.org%2F&h=x"), "https://b.org/");
    }

    #[test]
    fn sites() {
        assert_eq!(site("mail.google.com"), "google.com");
        assert_eq!(site("www.zapimoveis.com.br"), "zapimoveis.com.br");
        assert_eq!(site("teams.live.com"), "live.com");
        assert_eq!(host("https://user@Web.WhatsApp.com:443/x"), Some("web.whatsapp.com".into()));
        assert!(stays_inside("https://accounts.google.com/o", "https://mail.google.com/", &[]));
        assert!(stays_inside("https://docs.google.com/x", "https://mail.google.com/", &[]));
        assert!(!stays_inside("https://example.org/", "https://mail.google.com/", &[]));
        assert!(stays_inside("https://res.cdn.office.net/", "https://teams.live.com/", &["office.net".into()]));
    }

    #[test]
    fn reads_typed_addresses() {
        let g = "https://www.google.com/search?q=";
        assert_eq!(typed("g1.globo.com", g), "https://g1.globo.com");
        assert_eq!(typed("https://a.com/x?utm_source=y", g), "https://a.com/x");
        assert_eq!(typed("localhost:8080/app", g), "https://localhost:8080/app");
        assert_eq!(typed("10.0.0.250:8006", g), "https://10.0.0.250:8006");
        assert_eq!(typed("previsão do tempo", g), "https://www.google.com/search?q=previs%C3%A3o+do+tempo");
        assert_eq!(typed("rust", g), "https://www.google.com/search?q=rust");
        assert_eq!(typed("  ", g), "");
    }

    #[test]
    fn reads_unread_counts() {
        assert_eq!(unread("(3) WhatsApp"), Some(3));
        assert_eq!(unread("Caixa de entrada (12) - eu@gmail.com - Gmail"), Some(12));
        assert_eq!(unread("[5] Telegram"), Some(5));
        assert_eq!(unread("(1.234) Inbox"), Some(1234));
        assert_eq!(unread("Gmail (beta)"), None);
        assert_eq!(unread("WhatsApp"), None);
    }
}
