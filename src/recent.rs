//! Recently used files, the freedesktop way: `~/.local/share/recently-used.xbel`, which GTK and KDE programs write too.
//! The file manager lists it as its Recent place and adds what it opens to it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn path() -> PathBuf {
    std::env::var_os("XDG_DATA_HOME").map(PathBuf::from).unwrap_or_else(|| crate::fsops::home().join(".local/share")).join("recently-used.xbel")
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Seconds since the epoch of an xbel stamp (`2026-11-05T09:22:39.065000Z`, always UTC).
pub fn parse_stamp(s: &str) -> Option<u64> {
    let (date, time) = s.split_once('T')?;
    let mut d = date.split('-').map(|x| x.parse::<i64>().ok());
    let (y, mo, da) = (d.next()??, d.next()??, d.next()??);
    let time = time.trim_end_matches('Z');
    let mut t = time.split(':');
    let (h, mi) = (t.next()?.parse::<i64>().ok()?, t.next()?.parse::<i64>().ok()?);
    let sec = t.next().and_then(|x| x.split('.').next()?.parse::<i64>().ok()).unwrap_or(0);
    let secs = days_from_civil(y, mo, da) * 86400 + h * 3600 + mi * 60 + sec;
    u64::try_from(secs).ok()
}

pub fn stamp(secs: u64) -> String {
    let days = (secs / 86400) as i64;
    let rest = secs % 86400;
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.000000Z", rest / 3600, rest / 60 % 60, rest % 60)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// The value of attribute `name` inside one tag's text.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let key = format!(" {name}=\"");
    let at = tag.find(&key)? + key.len();
    let end = tag[at..].find('"')?;
    Some(&tag[at..at + end])
}

fn unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The local files of an xbel text with when each was last used, the newest first.
pub fn parse(xml: &str) -> Vec<(PathBuf, u64)> {
    let mut out: Vec<(PathBuf, u64)> = Vec::new();
    for part in xml.split("<bookmark ").skip(1) {
        let tag = &part[..part.find('>').unwrap_or(part.len())];
        let tag = format!(" {tag}");
        let Some(href) = attr(&tag, "href") else { continue };
        let Some(path) = crate::fsops::path_of_uri(&unescape(href)) else { continue };
        let when = ["visited", "modified", "added"].iter().filter_map(|k| attr(&tag, k).and_then(parse_stamp)).max().unwrap_or(0);
        match out.iter_mut().find(|(p, _)| *p == path) {
            Some(o) => o.1 = o.1.max(when),
            None => out.push((path, when)),
        }
    }
    out.sort_by(|a, b| b.1.cmp(&a.1));
    out
}

/// Recently used files that are still there, the newest first.
pub fn list(limit: usize) -> Vec<(PathBuf, u64)> {
    let text = std::fs::read_to_string(path()).unwrap_or_default();
    parse(&text).into_iter().filter(|(p, _)| p.exists()).take(limit).collect()
}

/// `xml` with `file` marked as used now by the program `app` (a new bookmark, or the old one's stamps moved on).
pub fn with_use(xml: &str, file: &Path, mime: &str, app: &str, exec: &str, at: u64) -> String {
    let href = escape(&crate::fsops::uri_of(file));
    let when = stamp(at);
    let needle = format!("href=\"{href}\"");
    if let Some(i) = xml.find(&needle) {
        // the bookmark's own tag: move its modified and visited stamps on
        let start = xml[..i].rfind("<bookmark ").unwrap_or(i);
        let end = i + xml[i..].find('>').unwrap_or(0);
        let mut tag = xml[start..end].to_string();
        for k in ["modified", "visited"] {
            let key = format!(" {k}=\"");
            match tag.find(&key) {
                Some(a) => {
                    let v0 = a + key.len();
                    let v1 = v0 + tag[v0..].find('"').unwrap_or(0);
                    tag.replace_range(v0..v1, &when);
                }
                None => tag.push_str(&format!(" {k}=\"{when}\"")),
            }
        }
        return format!("{}{}{}", &xml[..start], tag, &xml[end..]);
    }
    let entry = format!(
        "  <bookmark href=\"{href}\" added=\"{when}\" modified=\"{when}\" visited=\"{when}\">\n    <info>\n      <metadata owner=\"http://freedesktop.org\">\n        <mime:mime-type type=\"{}\"/>\n        <bookmark:applications>\n          <bookmark:application name=\"{}\" exec=\"{}\" modified=\"{when}\" count=\"1\"/>\n        </bookmark:applications>\n      </metadata>\n    </info>\n  </bookmark>\n",
        escape(mime),
        escape(app),
        escape(exec)
    );
    match xml.rfind("</xbel>") {
        Some(i) => format!("{}{}{}", &xml[..i], entry, &xml[i..]),
        None => format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<xbel version=\"1.0\" xmlns:bookmark=\"http://www.freedesktop.org/standards/desktop-bookmarks\" xmlns:mime=\"http://www.freedesktop.org/standards/shared-mime-info\">\n{entry}</xbel>\n"),
    }
}

/// Note that `file` was opened with program `app` (best effort: a file that cannot be written is left alone).
pub fn add(file: &Path, app: &str, exec: &str) {
    if file.is_dir() {
        return;
    }
    let p = path();
    let old = std::fs::read_to_string(&p).unwrap_or_default();
    let mime = crate::mimeapps::mime_of(file);
    let new = with_use(&old, file, &mime, app, exec, now());
    let tmp = p.with_extension("xbel.new");
    if std::fs::write(&tmp, new).is_ok() {
        let _ = std::fs::rename(&tmp, &p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stamps_go_both_ways() {
        let s = parse_stamp("2026-11-05T09:22:39.065000Z").unwrap();
        assert_eq!(stamp(s), "2026-11-05T09:22:39.000000Z");
        assert_eq!(parse_stamp("1970-01-01T00:00:00Z"), Some(0));
    }

    #[test]
    fn a_use_is_added_then_moved_on() {
        let f = Path::new("/tmp/a b.txt");
        let one = with_use("", f, "text/plain", "memrium-files", "memrium_files %u", 1000);
        let list = parse(&one);
        assert_eq!(list, vec![(f.to_path_buf(), 1000)]);
        let two = with_use(&one, f, "text/plain", "memrium-files", "x", 5000);
        assert_eq!(parse(&two), vec![(f.to_path_buf(), 5000)]);
        assert_eq!(two.matches("<bookmark ").count(), 1);
        let g = Path::new("/tmp/other");
        let three = with_use(&two, g, "text/plain", "a", "b", 7000);
        assert_eq!(parse(&three)[0].0, g.to_path_buf());
        assert!(three.trim_end().ends_with("</xbel>"));
    }
}
