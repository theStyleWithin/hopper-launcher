//! The "walls": open issues and recent discussions of the developer's own repositories, shown in the
//! empty space beside the apps on wide windows. Public GitHub data only, read when the launcher checks for
//! updates; the last reading is kept so the walls still show something offline.

use super::*;

const PER_REPO: usize = 30;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct WallItem {
    pub title: String,
    pub url: String,
    pub author: String,
    /// When it was last active, as GitHub wrote it (ISO 8601).
    pub when: String,
    pub number: Option<u64>,
    pub comments: Option<u64>,
    #[serde(default)]
    pub labels: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct RepoWall {
    pub id: String,
    pub name: String,
    pub repo: String,
    pub items: Vec<WallItem>,
    /// When this was read (Unix seconds).
    pub at: Option<u64>,
    /// False when GitHub couldn't be reached and these are the last saved items.
    pub fresh: bool,
    /// A quiet explanation, e.g. discussions are not turned on.
    pub note: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Walls {
    pub issues: Vec<RepoWall>,
    pub discussions: Vec<RepoWall>,
}

// ---------- reading GitHub ----------

fn web_base() -> String {
    std::env::var("HOPPER_WEB_BASE").unwrap_or_else(|_| "https://github.com".to_string())
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}
#[derive(Deserialize)]
struct RawUser {
    login: String,
}
#[derive(Deserialize)]
struct RawIssue {
    number: u64,
    title: String,
    html_url: String,
    updated_at: String,
    #[serde(default)]
    comments: u64,
    user: Option<RawUser>,
    #[serde(default)]
    labels: Vec<RawLabel>,
    /// Present when the "issue" is really a pull request, which is left out.
    pull_request: Option<serde_json::Value>,
}

static ISSUE_CACHE: std::sync::Mutex<Option<HashMap<String, (String, Vec<WallItem>)>>> = std::sync::Mutex::new(None);

/// Open issues, most recently active first. Asks "only if changed" so refreshing costs next to nothing.
fn fetch_issues(repo: &str) -> Result<Vec<WallItem>, SourceError> {
    if rate_limited_until().is_some() {
        return Err(SourceError::RateLimited); // GitHub said wait: asking again would only waste the next hour
    }
    let url = format!("{}/repos/{repo}/issues?state=open&sort=updated&per_page={PER_REPO}", api_base());
    let cached = ISSUE_CACHE.lock().ok().and_then(|c| c.as_ref().and_then(|m| m.get(&url).cloned()));
    let mut req = http().get(&url).set("User-Agent", "hopper-launcher").set("Accept", "application/vnd.github+json");
    if let Some((etag, _)) = &cached {
        req = req.set("If-None-Match", etag);
    }
    let resp = match req.call() {
        Ok(r) => r,
        Err(ureq::Error::Status(429, r)) => return Err(note_rate_limit(&r)),
        Err(ureq::Error::Status(403, r)) => return Err(forbidden(&r)),
        Err(ureq::Error::Status(404 | 410 | 451, _)) => return Err(SourceError::Gone),
        Err(ureq::Error::Status(code, _)) => return Err(SourceError::Unreachable(format!("GitHub error (HTTP {code})"))),
        Err(other) => return Err(SourceError::Unreachable(other.to_string())),
    };
    if resp.status() == 304 {
        return cached.map(|(_, i)| i).ok_or_else(|| SourceError::Other("nothing saved for unchanged issues".into()));
    }
    let etag = resp.header("etag").map(str::to_string);
    let raw: Vec<RawIssue> = resp.into_json().map_err(|e| SourceError::Other(e.to_string()))?;
    let items: Vec<WallItem> = raw
        .into_iter()
        .filter(|i| i.pull_request.is_none())
        .map(|i| WallItem {
            title: i.title,
            url: i.html_url,
            author: i.user.map(|u| u.login).unwrap_or_default(),
            when: i.updated_at,
            number: Some(i.number),
            comments: Some(i.comments),
            labels: i.labels.into_iter().map(|l| l.name).collect(),
        })
        .collect();
    if let (Some(etag), Ok(mut c)) = (etag, ISSUE_CACHE.lock()) {
        c.get_or_insert_with(HashMap::new).insert(url, (etag, items.clone()));
    }
    Ok(items)
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let Some(end) = rest.find(';').filter(|e| *e <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let ent = &rest[1..end];
        let ch = match ent {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => ent
                .strip_prefix('#')
                .and_then(|n| if let Some(h) = n.strip_prefix('x').or_else(|| n.strip_prefix('X')) { u32::from_str_radix(h, 16).ok() } else { n.parse().ok() })
                .and_then(char::from_u32),
        };
        match ch {
            Some(c) => {
                out.push(c);
                rest = &rest[end + 1..];
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

/// The text inside the first `<tag ...>text</tag>` of `block`.
fn tag_text(block: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}");
    let mut from = 0;
    while let Some(i) = block[from..].find(&open) {
        let start = from + i + open.len();
        match block[start..].chars().next() {
            Some('>') | Some(' ') | Some('\n') | Some('\t') | Some('\r') => {
                let gt = block[start..].find('>')? + start;
                if block[..gt].ends_with('/') {
                    return None;
                }
                let end = block[gt..].find(&format!("</{tag}>"))? + gt;
                let text = block[gt + 1..end].trim();
                let text = text.strip_prefix("<![CDATA[").and_then(|t| t.strip_suffix("]]>")).unwrap_or(text);
                return Some(unescape(text).trim().to_string());
            }
            _ => from = start,
        }
    }
    None
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let key = format!("{name}=\"");
    let i = tag.find(&key)? + key.len();
    let j = tag[i..].find('"')? + i;
    Some(unescape(&tag[i..j]))
}

/// Entries of a GitHub discussions Atom feed. Anything that does not look right is skipped, never guessed.
pub fn parse_atom(text: &str, repo: &str) -> Vec<WallItem> {
    let prefix = format!("https://github.com/{repo}/discussions/");
    let mut out = Vec::new();
    for chunk in text.split("<entry").skip(1) {
        let Some(end) = chunk.find("</entry>") else { continue };
        let block = &chunk[..end];
        let title = tag_text(block, "title").unwrap_or_default();
        let link = block
            .match_indices("<link")
            .filter_map(|(i, _)| block[i..].find('>').map(|e| &block[i..i + e]))
            .filter(|t| attr(t, "rel").map_or(true, |r| r == "alternate"))
            .find_map(|t| attr(t, "href"));
        let Some(url) = link else { continue };
        if title.is_empty() || !url.to_lowercase().starts_with(&prefix.to_lowercase()) {
            continue;
        }
        let when = tag_text(block, "updated").or_else(|| tag_text(block, "published")).unwrap_or_default();
        let author = block.find("<author").and_then(|i| tag_text(&block[i..], "name")).unwrap_or_default();
        let number = url.trim_end_matches('/').rsplit('/').next().and_then(|n| n.parse().ok());
        out.push(WallItem { title, url, author, when, number, comments: None, labels: vec![] });
        if out.len() >= PER_REPO {
            break;
        }
    }
    out
}

/// Recent discussions from the public Atom feed (GitHub's API has no plain way to list them). `None`
/// means the repository has discussions switched off (or none to read).
fn fetch_discussions(repo: &str) -> Result<Option<Vec<WallItem>>, SourceError> {
    let url = format!("{}/{repo}/discussions.atom", web_base());
    match http().get(&url).set("User-Agent", "hopper-launcher").set("Accept", "application/atom+xml").call() {
        Ok(resp) => {
            let mut text = String::new();
            resp.into_reader().take(2 << 20).read_to_string(&mut text).map_err(|e| SourceError::Other(e.to_string()))?;
            Ok(Some(parse_atom(&text, repo)))
        }
        Err(ureq::Error::Status(404 | 410, _)) => Ok(None),
        Err(ureq::Error::Status(429, r)) => Err(note_rate_limit(&r)),
        Err(ureq::Error::Status(code, _)) => Err(SourceError::Unreachable(format!("GitHub error (HTTP {code})"))),
        Err(other) => Err(SourceError::Unreachable(other.to_string())),
    }
}

// ---------- keeping the last reading ----------

fn walls_path() -> PathBuf {
    data_dir().join("walls.json")
}

fn load_walls() -> Walls {
    fs::read_to_string(walls_path()).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

fn save_walls(w: &Walls) {
    if fs::create_dir_all(data_dir()).is_ok() {
        if let Ok(text) = serde_json::to_string(w) {
            let _ = fs::write(walls_path(), text);
        }
    }
}

/// Every repository on the walls: the apps in the list, then the launcher itself.
fn wall_repos(apps: &[AppEntry]) -> Vec<(String, String, String)> {
    let mut v: Vec<(String, String, String)> = apps.iter().map(|a| (a.id.clone(), a.name.clone(), a.repo.clone())).collect();
    v.push(("hopper-launcher".into(), "Hopper Launcher".into(), self_repo()));
    v
}

/// Reads everything now. A repository that can't be read keeps what was saved for it, marked not fresh.
pub fn refresh_walls_blocking(apps: &[AppEntry], now: u64) -> Walls {
    let repos = wall_repos(apps);
    let saved = load_walls();
    let results: Vec<(Result<Vec<WallItem>, SourceError>, Result<Option<Vec<WallItem>>, SourceError>)> = std::thread::scope(|s| {
        let handles: Vec<_> = repos.iter().map(|(_, _, repo)| s.spawn(move || (fetch_issues(repo), fetch_discussions(repo)))).collect();
        handles.into_iter().filter_map(|h| h.join().ok()).collect()
    });
    let old = |list: &[RepoWall], id: &str| list.iter().find(|w| w.id == id).cloned();
    let mut walls = Walls::default();
    for ((id, name, repo), (iss, disc)) in repos.into_iter().zip(results) {
        let base = |items: Vec<WallItem>, fresh: bool, at: Option<u64>, note: Option<String>| RepoWall { id: id.clone(), name: name.clone(), repo: repo.clone(), items, at, fresh, note };
        walls.issues.push(match iss {
            Ok(items) => base(items, true, Some(now), None),
            Err(SourceError::Gone) => base(vec![], true, Some(now), Some("This repository can't be read right now.".into())),
            Err(_) => match old(&saved.issues, &id) {
                Some(w) => RepoWall { fresh: false, ..w },
                None => base(vec![], false, None, None),
            },
        });
        walls.discussions.push(match disc {
            Ok(Some(items)) => base(items, true, Some(now), None),
            Ok(None) => base(vec![], true, Some(now), Some("Discussions are not turned on here.".into())),
            Err(_) => match old(&saved.discussions, &id) {
                Some(w) => RepoWall { fresh: false, ..w },
                None => base(vec![], false, None, None),
            },
        });
    }
    save_walls(&walls);
    walls
}

/// A link may only open a page of one of the developer's own repositories.
pub fn link_allowed(url: &str, apps: &[AppEntry]) -> bool {
    if url.chars().any(|c| c.is_control() || c.is_whitespace() || c == '\\') {
        return false;
    }
    let Some(path) = url.strip_prefix("https://github.com/") else { return false };
    let mut parts = path.split('/');
    let (Some(owner), Some(name)) = (parts.next(), parts.next()) else { return false };
    let full = format!("{owner}/{name}").to_lowercase();
    wall_repos(apps).iter().any(|(_, _, r)| r.to_lowercase() == full)
}

#[tauri::command]
pub async fn wall_items() -> Result<Walls, String> {
    if !load_settings().show_walls {
        return Ok(Walls::default());
    }
    let apps = load_manifest()?;
    tauri::async_runtime::spawn_blocking(move || refresh_walls_blocking(&apps, unix_now())).await.map_err(|e| e.to_string())
}

/// The last saved reading, with no request at all (used so the walls appear instantly at start).
#[tauri::command]
pub fn saved_walls() -> Walls {
    if load_settings().show_walls { load_walls() } else { Walls::default() }
}

#[tauri::command]
pub fn open_link(url: String) -> Result<(), String> {
    let apps = load_manifest()?;
    if !link_allowed(&url, &apps) {
        return Err("That link isn't one of your repositories, so it wasn't opened.".into());
    }
    open_with_system(&url)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FEED: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<feed xmlns="http://www.w3.org/2005/Atom" xml:lang="en-US">
  <id>tag:github.com,2008:/theStyleWithin/hopcast/discussions</id>
  <title>Recent Discussions · theStyleWithin/hopcast</title>
  <updated>2026-11-01T10:00:00Z</updated>
  <entry>
    <id>tag:github.com,2008:Discussion/1</id>
    <published>2026-10-30T09:00:00Z</published>
    <updated>2026-11-01T10:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/theStyleWithin/hopcast/discussions/12"/>
    <title>Can we get &quot;dark&quot; chat &amp; emotes?</title>
    <author><name>viewer42</name><uri>https://github.com/viewer42</uri></author>
    <content type="html">&lt;p&gt;body&lt;/p&gt;</content>
  </entry>
  <entry>
    <id>tag:github.com,2008:Discussion/2</id>
    <updated>2026-10-29T08:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://evil.example/phish"/>
    <title>Not one of ours</title>
    <author><name>x</name></author>
  </entry>
  <entry>
    <id>tag:github.com,2008:Discussion/3</id>
    <updated>2026-10-28T08:00:00Z</updated>
    <link rel="alternate" type="text/html" href="https://github.com/theStyleWithin/hopcast/discussions/7"/>
    <title>Plain title</title>
    <author><name>gamehopper</name></author>
  </entry>
</feed>"#;

    #[test]
    fn the_discussions_feed_is_read_carefully() {
        let items = parse_atom(FEED, "theStyleWithin/hopcast");
        assert_eq!(items.len(), 2, "a link to another site must be skipped: {items:?}");
        assert_eq!(items[0].title, "Can we get \"dark\" chat & emotes?");
        assert_eq!((items[0].author.as_str(), items[0].number, items[0].when.as_str()), ("viewer42", Some(12), "2026-11-01T10:00:00Z"));
        assert_eq!(items[1].title, "Plain title");
        // A feed for another repository yields nothing, and junk yields nothing.
        assert!(parse_atom(FEED, "someone/else").is_empty());
        assert!(parse_atom("<html>not a feed", "a/b").is_empty());
        assert!(parse_atom("", "a/b").is_empty());
    }

    #[test]
    fn entities_are_decoded_without_going_wrong() {
        assert_eq!(unescape("a &amp;lt; b &#39;x&#x41; &bogus; &"), "a &lt; b 'xA &bogus; &");
    }

    #[test]
    fn only_the_developers_own_repositories_can_be_opened() {
        let apps = vec![AppEntry { id: "hopcast".into(), name: "HopCast".into(), tagline: String::new(), repo: "theStyleWithin/hopcast".into(), accent: None, untested_on: vec![], archive: None }];
        assert!(link_allowed("https://github.com/theStyleWithin/hopcast/issues/3", &apps));
        assert!(link_allowed("https://github.com/thestylewithin/HOPCAST/discussions/1", &apps));
        assert!(link_allowed("https://github.com/theStyleWithin/hopper-launcher/issues/1", &apps));
        assert!(!link_allowed("https://github.com/someoneelse/repo/issues/1", &apps));
        assert!(!link_allowed("https://evil.example/theStyleWithin/hopcast", &apps));
        assert!(!link_allowed("http://github.com/theStyleWithin/hopcast", &apps));
        assert!(!link_allowed("https://github.com/theStyleWithin/hopcast/issues/1 && calc", &apps));
        assert!(!link_allowed("file:///etc/passwd", &apps));
    }
}
