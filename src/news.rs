//! RSS / Atom feed reader for the news widget. Uses the system URL cache
//! downloader, so there is no HTTP or TLS stack compiled into the app.

use windows::core::*;
use windows::Win32::System::Com::Urlmon::URLDownloadToCacheFileW;

use crate::sysinfo::days_from_civil;

#[derive(Clone)]
pub struct NewsItem {
    pub title: String,
    pub summary: String,
    pub link: String,
    /// Unix seconds, 0 when the feed has no usable date.
    pub ts: i64,
}

pub fn fetch(url: &str) -> Vec<NewsItem> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Vec::new();
    }
    let mut path = [0u16; 1024];
    let ok = unsafe { URLDownloadToCacheFileW(None, &HSTRING::from(url), &mut path, 0, None).is_ok() };
    if !ok {
        return Vec::new();
    }
    let len = path.iter().position(|&c| c == 0).unwrap_or(0);
    let file = String::from_utf16_lossy(&path[..len]);
    match std::fs::read(file) {
        Ok(bytes) => parse(&String::from_utf8_lossy(&bytes)),
        Err(_) => Vec::new(),
    }
}

pub fn parse(xml: &str) -> Vec<NewsItem> {
    let (open, close) = if xml.contains("<item") { ("<item", "</item>") } else { ("<entry", "</entry>") };
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find(open) {
        let body = &rest[start..];
        let Some(end) = body.find(close) else { break };
        let item = &body[..end];
        rest = &body[end + close.len()..];

        let title = clean(&tag(item, "title").unwrap_or_default());
        if title.is_empty() {
            continue;
        }
        let summary = clean(
            &tag(item, "description").or_else(|| tag(item, "summary")).or_else(|| tag(item, "content")).unwrap_or_default(),
        );
        let link = tag(item, "link").map(|l| clean(&l)).filter(|l| l.starts_with("http")).or_else(|| attr(item, "link", "href"));
        let date = tag(item, "pubDate").or_else(|| tag(item, "updated")).or_else(|| tag(item, "published"));
        out.push(NewsItem {
            title,
            summary: summary.chars().take(220).collect(),
            link: link.unwrap_or_default(),
            ts: date.and_then(|d| parse_date(d.trim())).unwrap_or(0),
        });
        if out.len() >= 12 {
            break;
        }
    }
    out
}

fn tag(s: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    let mut from = 0;
    loop {
        let i = s[from..].find(&open)? + from;
        let after = &s[i + open.len()..];
        // Must be exactly this tag, not a longer one such as <linkfoo>.
        match after.chars().next() {
            Some('>') | Some(' ') | Some('\t') | Some('\n') | Some('\r') => {}
            _ => {
                from = i + open.len();
                continue;
            }
        }
        let gt = after.find('>')?;
        if after[..gt].ends_with('/') {
            return None;
        }
        let content = &after[gt + 1..];
        let end = content.find(&format!("</{name}>"))?;
        return Some(content[..end].to_string());
    }
}

fn attr(s: &str, name: &str, attr: &str) -> Option<String> {
    let i = s.find(&format!("<{name} "))?;
    let t = &s[i..];
    let t = &t[..t.find('>')?];
    let a = t.find(&format!("{attr}=\""))? + attr.len() + 2;
    let e = t[a..].find('"')?;
    Some(t[a..a + e].to_string())
}

/// Strips CDATA wrappers, markup and entities, and collapses whitespace.
fn clean(s: &str) -> String {
    let s = s.replace("<![CDATA[", "").replace("]]>", "");
    let s = decode(&s);
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' if in_tag => {
                in_tag = false;
                out.push(' ');
            }
            _ if !in_tag => out.push(c),
            _ => {}
        }
    }
    decode(&out).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(semi) = rest[..rest.len().min(12)].find(';') else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..semi];
        let rep = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some(' '),
            _ => ent
                .strip_prefix("#x")
                .and_then(|h| u32::from_str_radix(h, 16).ok())
                .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                .and_then(char::from_u32),
        };
        match rep {
            Some(c) => {
                out.push(c);
                rest = &rest[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Parses RFC 822 ("Tue, 10 Nov 2026 14:00:00 +0000") and ISO 8601 dates.
fn parse_date(s: &str) -> Option<i64> {
    let b = s.as_bytes();
    if b.len() >= 19 && b[4] == b'-' && b[10] == b'T' {
        let n = |a: usize, z: usize| s.get(a..z)?.parse::<i64>().ok();
        let secs = days_from_civil(n(0, 4)?, n(5, 7)?, n(8, 10)?) * 86400 + n(11, 13)? * 3600 + n(14, 16)? * 60 + n(17, 19)?;
        return Some(secs - iso_offset(&s[19..]));
    }
    let parts: Vec<&str> = s.split_whitespace().collect();
    let p = if parts.first().map(|p| p.ends_with(',')).unwrap_or(false) { &parts[1..] } else { &parts[..] };
    if p.len() < 4 {
        return None;
    }
    let day: i64 = p[0].parse().ok()?;
    let mon = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| p[1].to_ascii_lowercase().starts_with(m))? as i64
        + 1;
    let year: i64 = p[2].parse().ok()?;
    let mut hms = p[3].split(':').map(|x| x.parse::<i64>().unwrap_or(0));
    let secs = days_from_civil(year, mon, day) * 86400
        + hms.next().unwrap_or(0) * 3600
        + hms.next().unwrap_or(0) * 60
        + hms.next().unwrap_or(0);
    let off = match p.get(4) {
        Some(z) if z.len() == 5 && (z.starts_with('+') || z.starts_with('-')) => {
            let h: i64 = z[1..3].parse().unwrap_or(0);
            let m: i64 = z[3..5].parse().unwrap_or(0);
            (h * 3600 + m * 60) * if z.starts_with('-') { -1 } else { 1 }
        }
        _ => 0,
    };
    Some(secs - off)
}

fn iso_offset(tail: &str) -> i64 {
    let t = tail.trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    if t.len() >= 6 && (t.starts_with('+') || t.starts_with('-')) {
        let h: i64 = t[1..3].parse().unwrap_or(0);
        let m: i64 = t[4..6].parse().unwrap_or(0);
        return (h * 3600 + m * 60) * if t.starts_with('-') { -1 } else { 1 };
    }
    0
}

pub fn age(ts: i64) -> String {
    if ts <= 0 {
        return String::new();
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let d = (now - ts).max(0);
    if d < 3600 {
        format!("{}m", (d / 60).max(1))
    } else if d < 86400 {
        format!("{}h", d / 3600)
    } else {
        format!("{}d", d / 86400)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rss() {
        let xml = r#"<rss><channel><title>Feed</title>
          <item><title><![CDATA[Webb &amp; Hubble]]></title><link>https://example.com/a</link>
          <description>&lt;p&gt;Hello <b>world</b>&lt;/p&gt;</description>
          <pubDate>Tue, 10 Nov 2026 14:00:00 +0000</pubDate></item>
          <item><title>Second</title><link>https://example.com/b</link></item></channel></rss>"#;
        let items = parse(xml);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].title, "Webb & Hubble");
        assert_eq!(items[0].summary, "Hello world");
        assert_eq!(items[0].link, "https://example.com/a");
        assert_eq!(items[0].ts, days_from_civil(2026, 11, 10) * 86400 + 14 * 3600);
    }

    #[test]
    fn atom() {
        let xml = r#"<feed><entry><title>One</title><link href="https://example.com/1"/>
          <updated>2026-11-10T14:00:00+02:00</updated><summary>S</summary></entry></feed>"#;
        let items = parse(xml);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].link, "https://example.com/1");
        assert_eq!(items[0].ts, days_from_civil(2026, 11, 10) * 86400 + 12 * 3600);
    }
}
