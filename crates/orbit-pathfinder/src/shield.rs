//! The block lists: EasyList and EasyPrivacy (or the lists the config
//! names), turned from the Adblock format into WebKit's content-blocker
//! rules and compiled into the `UserContentFilterStore`, then put on every
//! page that blocks (all of them, unless a service says `block: false`).
//!
//! Each list becomes two filters: its network rules (requests blocked) and
//! its element-hiding rules (CSS), so a selector WebKit will not take
//! leaves the blocking in place. The compiled filters load at start; the
//! lists are fetched again when older than four days, and recompiled only
//! when their text changed.
//!
//! What the Adblock format has and WebKit does not — regular-expression
//! rules, `$redirect`, `$csp`, `$removeparam`, extended selectors
//! (`:has-text`, `:-abp-…`) — is left out.

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use gtk::{gio, glib, prelude::*};
use serde_json::{Value, json};

/// The lists when the config names none.
pub const DEFAULT_LISTS: &[&str] = &["https://easylist.to/easylist/easylist.txt", "https://easylist.to/easylist/easyprivacy.txt"];

/// Bumped when the conversion changes, so the lists compile again.
const CONVERTER: u32 = 1;
/// Lists are fetched again after this long (EasyList's own "Expires").
const STALE: Duration = Duration::from_secs(4 * 24 * 3600);
/// How often a running browser looks for newer lists.
const CHECK: Duration = Duration::from_secs(6 * 3600);
/// A list larger than this is not one.
const LIST_MAX: u64 = 30 * 1024 * 1024;

/// The resource types a rule without any applies to: all but the page
/// itself (ABP's default), so a pattern never stops a page you opened.
const DEFAULT_TYPES: &[&str] = &["image", "style-sheet", "script", "font", "raw", "svg-document", "media", "ping", "other"];

/// The block lists and the pages they go on.
pub struct Shield {
    store: webkit6::UserContentFilterStore,
    /// Where the lists' text is kept.
    dir: PathBuf,
    lists: Vec<String>,
    /// The filters in force, by identifier.
    filters: RefCell<BTreeMap<String, webkit6::UserContentFilter>>,
    managers: RefCell<Vec<glib::WeakRef<webkit6::UserContentManager>>>,
    /// Pages waiting for the filters of the last run to load (a page loaded
    /// before them would go unprotected), and how many loads are still out.
    waiting: RefCell<Vec<Box<dyn FnOnce()>>>,
    loading: std::cell::Cell<usize>,
}

impl Shield {
    /// Under `cache`, the lists' text and the filters compiled from it
    /// (some 90 MB for the two default lists, made again when missing).
    pub fn new(cache: &Path, lists: Vec<String>) -> Rc<Self> {
        let store_dir = cache.join("filters");
        let _ = std::fs::create_dir_all(&store_dir);
        let shield = Rc::new(Self {
            store: webkit6::UserContentFilterStore::new(&store_dir.to_string_lossy()),
            dir: cache.join("lists"),
            lists,
            filters: RefCell::default(),
            managers: RefCell::default(),
            waiting: RefCell::default(),
            loading: std::cell::Cell::new(0),
        });
        shield.load();
        shield.refresh();
        let weak = Rc::downgrade(&shield);
        glib::timeout_add_local(CHECK, move || match weak.upgrade() {
            Some(shield) => {
                shield.refresh();
                glib::ControlFlow::Continue
            }
            None => glib::ControlFlow::Break,
        });
        shield
    }

    /// Puts the filters on `manager`'s pages, now and as they change.
    pub fn attach(&self, manager: &webkit6::UserContentManager) {
        for filter in self.filters.borrow().values() {
            manager.add_filter(filter);
        }
        let mut managers = self.managers.borrow_mut();
        managers.retain(|m| m.upgrade().is_some());
        managers.push(manager.downgrade());
    }

    /// Runs `f` once the filters of the last run are in force (or failed to
    /// load: the first run has none).
    pub fn when_ready(&self, f: impl FnOnce() + 'static) {
        if self.loading.get() == 0 {
            f();
        } else {
            self.waiting.borrow_mut().push(Box::new(f));
        }
    }

    /// The identifiers of a list's two filters.
    fn ids(&self, url: &str) -> (String, String) {
        let id = list_id(url);
        (format!("{id}-block"), format!("{id}-hide"))
    }

    /// The filters compiled in an earlier run, straight away.
    fn load(self: &Rc<Self>) {
        for url in &self.lists {
            let (block, hide) = self.ids(url);
            for id in [block, hide] {
                self.loading.set(self.loading.get() + 1);
                let weak = Rc::downgrade(self);
                self.store.load(&id, gio::Cancellable::NONE, move |result| {
                    let Some(shield) = weak.upgrade() else { return };
                    if let Ok(filter) = result {
                        shield.put(filter);
                    }
                    shield.loading.set(shield.loading.get() - 1);
                    if shield.loading.get() == 0 {
                        let waiting = std::mem::take(&mut *shield.waiting.borrow_mut());
                        for f in waiting {
                            f();
                        }
                    }
                });
            }
        }
    }

    /// Fetches the lists that are old and compiles those whose text
    /// changed, in a thread; the store compiles on WebKit's own.
    fn refresh(self: &Rc<Self>) {
        let jobs: Vec<(String, PathBuf, PathBuf)> = self
            .lists
            .iter()
            .map(|url| {
                let id = list_id(url);
                (url.clone(), self.dir.join(format!("{id}.txt")), self.dir.join(format!("{id}.stamp")))
            })
            .collect();
        let dir = self.dir.clone();
        let weak = Rc::downgrade(self);
        crate::ui::background(
            move || {
                let _ = std::fs::create_dir_all(&dir);
                jobs.into_iter().filter_map(|(url, text, stamp)| prepare(&url, &text, &stamp).map(|r| (url, stamp, r))).collect::<Vec<_>>()
            },
            move |ready: Vec<(String, PathBuf, Rules)>| {
                let Some(shield) = weak.upgrade() else { return };
                for (url, stamp_file, rules) in ready {
                    let (block, hide) = shield.ids(&url);
                    shield.compile(block, rules.block, None);
                    shield.compile(hide, rules.hide, Some((stamp_file, rules.stamp)));
                }
            },
        );
    }

    /// Compiles `json` as filter `id` and puts it in force; `stamp` is
    /// written once it compiled (the list's second filter carries it).
    fn compile(self: &Rc<Self>, id: String, json: String, stamp: Option<(PathBuf, String)>) {
        let weak = Rc::downgrade(self);
        let bytes = glib::Bytes::from_owned(json.into_bytes());
        self.store.save(&id.clone(), &bytes, gio::Cancellable::NONE, move |result| match result {
            Ok(filter) => {
                if let Some(shield) = weak.upgrade() {
                    shield.put(filter);
                }
                if let Some((file, text)) = stamp {
                    let _ = std::fs::write(file, text);
                }
            }
            Err(err) => log::warn!("block list {id}: {err}"),
        });
    }

    /// Puts `filter` on every page, in place of the one of the same name.
    fn put(&self, filter: webkit6::UserContentFilter) {
        let Some(id) = filter.identifier().map(|s| s.to_string()) else { return };
        let managers: Vec<_> = self.managers.borrow().iter().filter_map(|m| m.upgrade()).collect();
        for manager in &managers {
            manager.remove_filter_by_id(&id);
            manager.add_filter(&filter);
        }
        self.filters.borrow_mut().insert(id, filter);
    }
}

/// "https://easylist.to/easylist/easyprivacy.txt" → "easyprivacy".
fn list_id(url: &str) -> String {
    let name = url.trim_end_matches('/').rsplit('/').next().unwrap_or(url);
    crate::config::slug(name.trim_end_matches(".txt"))
}

/// A list's rules, ready to compile, and the stamp that says which text
/// they came from.
struct Rules {
    block: String,
    hide: String,
    stamp: String,
}

/// Fetches the list when it is old or missing and converts it when its text
/// is not the one last compiled. Nothing to do: `None`.
fn prepare(url: &str, text_file: &Path, stamp_file: &Path) -> Option<Rules> {
    let age = std::fs::metadata(text_file).and_then(|m| m.modified()).ok().and_then(|t| SystemTime::now().duration_since(t).ok());
    if age.is_none_or(|a| a > STALE) {
        match fetch(url) {
            Ok(text) => {
                let tmp = text_file.with_extension("tmp");
                if std::fs::write(&tmp, &text).and_then(|()| std::fs::rename(&tmp, text_file)).is_err() {
                    log::warn!("{}: not written", text_file.display());
                }
            }
            // The copy at hand still serves.
            Err(err) => log::warn!("block list {url}: {err}"),
        }
    }
    let text = std::fs::read_to_string(text_file).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (CONVERTER, &text).hash(&mut hasher);
    let stamp = format!("{:016x}", hasher.finish());
    if std::fs::read_to_string(stamp_file).ok().as_deref() == Some(&stamp) {
        return None;
    }
    let (block, hide) = convert(&text);
    Some(Rules { block: block.to_string(), hide: hide.to_string(), stamp })
}

fn fetch(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(60))).build().into();
    agent.get(url).call().map_err(|e| e.to_string())?.body_mut().with_config().limit(LIST_MAX).read_to_string().map_err(|e| e.to_string())
}

/// An Adblock list as WebKit rules: (network rules, element-hiding rules).
pub fn convert(text: &str) -> (Value, Value) {
    let mut block = Vec::new();
    let mut allow = Vec::new();
    // Pages where no rule applies at all, and where no element is hidden.
    let mut whole_sites = Vec::new();
    let mut hide_off = Vec::new();
    let mut generic_off = Vec::new();
    // Generic selectors, by the sites that are exempt from them.
    let mut generic: BTreeMap<Vec<String>, Vec<String>> = BTreeMap::new();
    let mut exempt: HashMap<String, Vec<String>> = HashMap::new();
    let mut specific = Vec::new();

    let lines: Vec<&str> = text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('!') && !l.starts_with('[')).collect();
    // Exceptions to hiding first: they decide how the generic rules go out.
    for line in &lines {
        if let Some((domains, selector)) = line.split_once("#@#")
            && !domains.is_empty()
            && let Some(domains) = domain_list(domains, ',')
        {
            exempt.entry(selector.to_owned()).or_default().extend(domains.0);
        }
    }
    for line in &lines {
        if line.contains("#@#") || line.contains("#?#") || line.contains("#$#") || line.contains("#%#") {
            continue;
        }
        if let Some((domains, selector)) = line.split_once("##") {
            if !plain_selector(selector) {
                continue;
            }
            if domains.is_empty() {
                let mut off = exempt.get(selector).cloned().unwrap_or_default();
                off.sort();
                off.dedup();
                generic.entry(off).or_default().push(selector.to_owned());
            } else if let Some((on, off)) = domain_list(domains, ',') {
                let mut trigger = json!({ "url-filter": ".*" });
                if !on.is_empty() {
                    trigger["if-domain"] = json!(on);
                } else if !off.is_empty() {
                    trigger["unless-domain"] = json!(off);
                }
                specific.push(json!({ "trigger": trigger, "action": { "type": "css-display-none", "selector": selector } }));
            }
            continue;
        }
        let Some(rule) = network(line) else { continue };
        match rule {
            Net::Block(r) => block.push(r),
            Net::Allow(r) => allow.push(r),
            Net::Site(d) => whole_sites.extend(d),
            Net::NoHiding(d) => hide_off.extend(d),
            Net::NoGeneric(d) => generic_off.extend(d),
        }
    }

    let ignore = |domains: Vec<String>| {
        (!domains.is_empty()).then(|| json!({ "trigger": { "url-filter": ".*", "if-domain": domains }, "action": { "type": "ignore-previous-rules" } }))
    };
    let mut net = block;
    net.extend(allow);
    net.extend(ignore(whole_sites.clone()));

    let mut hide = Vec::new();
    for (off, selectors) in generic {
        // Many selectors to a rule: fewer rules compile faster.
        for chunk in selectors.chunks(250) {
            let mut trigger = json!({ "url-filter": ".*" });
            if !off.is_empty() {
                trigger["unless-domain"] = json!(off);
            }
            hide.push(json!({ "trigger": trigger, "action": { "type": "css-display-none", "selector": chunk.join(", ") } }));
        }
    }
    hide.extend(ignore(generic_off));
    hide.extend(specific);
    hide_off.extend(whole_sites);
    hide.extend(ignore(hide_off));
    (Value::Array(net), Value::Array(hide))
}

/// A network line, as WebKit has it.
enum Net {
    Block(Value),
    /// An exception to the rules before it (`@@`).
    Allow(Value),
    /// `@@||site^$document`: nothing is blocked on these sites' pages.
    Site(Vec<String>),
    /// `$elemhide`: nothing hidden on these sites' pages.
    NoHiding(Vec<String>),
    /// `$generichide`: only the rules naming these sites hide there.
    NoGeneric(Vec<String>),
}

fn network(line: &str) -> Option<Net> {
    let (allow, line) = match line.strip_prefix("@@") {
        Some(rest) => (true, rest),
        None => (false, line),
    };
    // Regular expressions: WebKit's are a subset; left out.
    if line.starts_with('/') && line.split('$').next().is_some_and(|p| p.len() > 1 && p.ends_with('/')) {
        return None;
    }
    let (pattern, options) = match line.rfind('$') {
        Some(i) if !line[i + 1..].contains('/') => (&line[..i], &line[i + 1..]),
        _ => (line, ""),
    };
    let mut types: Option<Vec<&str>> = None;
    let mut not_types: Vec<&str> = Vec::new();
    let mut trigger = serde_json::Map::new();
    let (mut document, mut elemhide, mut generichide) = (false, false, false);
    for option in options.split(',').filter(|o| !o.is_empty()) {
        let (name, value) = option.split_once('=').unwrap_or((option, ""));
        let (negated, name) = match name.strip_prefix('~') {
            Some(n) => (true, n),
            None => (false, name),
        };
        let webkit = match name {
            "script" => Some("script"),
            "image" => Some("image"),
            "stylesheet" => Some("style-sheet"),
            "font" => Some("font"),
            "media" => Some("media"),
            "xmlhttprequest" => Some("raw"),
            "websocket" => Some("websocket"),
            "ping" => Some("ping"),
            "other" | "object" => Some("other"),
            "popup" => Some("popup"),
            // A frame inside a page; WebKit's "document" with no load
            // context would also stop the page itself.
            "subdocument" => None,
            _ => None,
        };
        match name {
            "third-party" | "3p" => {
                trigger.insert("load-type".into(), json!([if negated { "first-party" } else { "third-party" }]));
            }
            "first-party" | "1p" => {
                trigger.insert("load-type".into(), json!([if negated { "third-party" } else { "first-party" }]));
            }
            "domain" => {
                let (on, off) = domain_list(value, '|')?;
                if !on.is_empty() {
                    trigger.insert("if-domain".into(), json!(on));
                } else if !off.is_empty() {
                    trigger.insert("unless-domain".into(), json!(off));
                }
            }
            "match-case" => {
                trigger.insert("url-filter-is-case-sensitive".into(), json!(true));
            }
            "document" if allow && !negated => document = true,
            // A site that is only ads or scams: its pages too.
            "document" if !negated => types.get_or_insert_with(Vec::new).push("document"),
            "elemhide" | "ehide" if allow => elemhide = true,
            "generichide" | "ghide" if allow => generichide = true,
            "important" | "all" => {}
            "subdocument" | "document" if negated => {}
            // Frames only: no rule WebKit can say without the page too.
            "subdocument" => {
                if types.is_none() {
                    types = Some(Vec::new());
                }
            }
            _ if webkit.is_some() => {
                let t = webkit.unwrap_or_default();
                if negated {
                    not_types.push(t);
                } else {
                    types.get_or_insert_with(Vec::new).push(t);
                }
            }
            // $redirect, $csp, $removeparam, $rewrite, $header, $method…:
            // nothing WebKit can do.
            _ => return None,
        }
    }

    if document || elemhide || generichide {
        // Whole-site exceptions: the sites the pattern names.
        let site = pattern.trim_start_matches("||").trim_start_matches('|').trim_end_matches('^').trim_end_matches('/');
        let mut sites = match trigger.get("if-domain").and_then(Value::as_array) {
            Some(d) => d.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect(),
            None => Vec::new(),
        };
        if pattern.starts_with("||") && !site.is_empty() && site.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            sites.push(format!("*{}", site.to_ascii_lowercase()));
        }
        if sites.is_empty() {
            return None;
        }
        return Some(if document {
            Net::Site(sites)
        } else if elemhide {
            Net::NoHiding(sites)
        } else {
            Net::NoGeneric(sites)
        });
    }

    let types: Vec<&str> = match types {
        Some(t) if t.is_empty() => return None,
        Some(t) => t,
        None => DEFAULT_TYPES.iter().copied().filter(|t| !not_types.contains(t)).collect(),
    };
    trigger.insert("url-filter".into(), json!(url_filter(pattern)?));
    trigger.insert("resource-type".into(), json!(types));
    let action = if allow { "ignore-previous-rules" } else { "block" };
    let rule = json!({ "trigger": trigger, "action": { "type": action } });
    Some(if allow { Net::Allow(rule) } else { Net::Block(rule) })
}

/// An Adblock pattern as the regular expression WebKit matches URLs with.
fn url_filter(pattern: &str) -> Option<String> {
    if !pattern.is_ascii() {
        return None;
    }
    let mut out = String::with_capacity(pattern.len() + 24);
    let mut rest = pattern;
    if let Some(r) = rest.strip_prefix("||") {
        // The host or any host under it, on any scheme.
        out.push_str("^[a-z][a-z0-9.+-]*://([^/?#]*\\.)?");
        rest = r;
    } else if let Some(r) = rest.strip_prefix('|') {
        out.push('^');
        rest = r;
    }
    let end = rest.strip_suffix('|');
    if let Some(r) = end {
        rest = r;
    }
    let chars: Vec<char> = rest.chars().collect();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '*' => out.push_str(".*"),
            // The separator: any of these, or the end of the address.
            '^' if i + 1 == chars.len() => out.push_str("([/:?&=].*)?$"),
            '^' => out.push_str("[/:?&=]"),
            '.' | '+' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '\\' | '$' | '|' => {
                out.push('\\');
                out.push(*c);
            }
            c => out.push(*c),
        }
    }
    if end.is_some() {
        out.push('$');
    }
    let out = out.trim_start_matches(".*").to_owned();
    Some(if out.is_empty() { ".*".into() } else { out })
}

/// "a.com,~b.a.com" → (["*a.com"], ["*b.a.com"]): the sites and their
/// subdomains, as WebKit names them. `None` for a name WebKit would refuse.
fn domain_list(list: &str, sep: char) -> Option<(Vec<String>, Vec<String>)> {
    let (mut on, mut off) = (Vec::new(), Vec::new());
    for d in list.split(sep).map(str::trim).filter(|d| !d.is_empty()) {
        let (neg, d) = match d.strip_prefix('~') {
            Some(d) => (true, d),
            None => (false, d),
        };
        // "google.*" (any country) and names outside ASCII: not WebKit's.
        if d.ends_with(".*") || !d.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-') {
            continue;
        }
        let d = format!("*{}", d.to_ascii_lowercase());
        if neg { off.push(d) } else { on.push(d) }
    }
    (!on.is_empty() || !off.is_empty()).then_some((on, off))
}

/// A selector WebKit's CSS knows: the extended ones of the ad blockers
/// (`:has-text`, `:-abp-contains`, `:xpath`…) are not.
fn plain_selector(selector: &str) -> bool {
    const EXTENDED: &[&str] = &[
        ":-abp-", ":has-text(", ":contains(", ":xpath(", ":style(", ":upward(", ":remove(", ":matches-", ":min-text-length(",
        ":watch-attr(", ":others(", ":if(", ":if-not(", ":nth-ancestor(", ":remove-attr(", ":remove-class(", ":shadow(",
    ];
    !selector.is_empty() && selector.is_ascii() && !selector.starts_with('+') && !EXTENDED.iter().any(|e| selector.contains(e))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(line: &str) -> Value {
        match network(line) {
            Some(Net::Block(r) | Net::Allow(r)) => r,
            _ => Value::Null,
        }
    }

    #[test]
    fn patterns() {
        assert_eq!(url_filter("||ads.example.com^").unwrap(), "^[a-z][a-z0-9.+-]*://([^/?#]*\\.)?ads\\.example\\.com([/:?&=].*)?$");
        assert_eq!(url_filter("/banner/*/img^").unwrap(), "/banner/.*/img([/:?&=].*)?$");
        assert_eq!(url_filter("|https://x.org/a.js|").unwrap(), "^https://x\\.org/a\\.js$");
        assert_eq!(url_filter("*").unwrap(), ".*");
        assert_eq!(url_filter("-ad-banner."), Some("-ad-banner\\.".into()));
        assert_eq!(url_filter("/anúncio/"), None);
    }

    #[test]
    fn options() {
        let r = one("||tracker.net^$third-party,script,domain=a.com|b.org");
        assert_eq!(r["trigger"]["load-type"], json!(["third-party"]));
        assert_eq!(r["trigger"]["resource-type"], json!(["script"]));
        assert_eq!(r["trigger"]["if-domain"], json!(["*a.com", "*b.org"]));
        assert_eq!(r["action"]["type"], "block");
        let r = one("@@||cdn.site.com/ads.js$script");
        assert_eq!(r["action"]["type"], "ignore-previous-rules");
        // Everything but images.
        let r = one("/pixel.$~image");
        assert!(!r["trigger"]["resource-type"].as_array().unwrap().contains(&json!("image")));
        // What WebKit cannot do is dropped.
        assert!(network("||x.com^$redirect=noop.js").is_none());
        assert!(network("/^https?:\\/\\/ad/$script").is_none());
        assert!(network("||frames.com^$subdocument").is_none());
        assert!(matches!(network("@@||bank.com^$document"), Some(Net::Site(s)) if s == ["*bank.com"]));
        assert_eq!(one("||scam.example^$document")["trigger"]["resource-type"], json!(["document"]));
        assert!(matches!(network("@@||news.com^$generichide"), Some(Net::NoGeneric(_))));
    }

    #[test]
    fn hiding() {
        let list = "! comment\n##.ad-box\n##.banner\nexample.com#@#.banner\nsite.org,~m.site.org##.promo\n##div:has-text(Sponsored)\n||ads.com^\n";
        let (net, hide) = convert(list);
        assert_eq!(net.as_array().unwrap().len(), 1);
        let hide = hide.as_array().unwrap();
        // .ad-box for all; .banner but on example.com; .promo on site.org.
        assert_eq!(hide.len(), 3);
        assert!(hide.iter().any(|r| r["action"]["selector"] == ".ad-box"));
        assert!(hide.iter().any(|r| r["action"]["selector"] == ".banner" && r["trigger"]["unless-domain"] == json!(["*example.com"])));
        assert!(hide.iter().any(|r| r["action"]["selector"] == ".promo" && r["trigger"]["if-domain"] == json!(["*site.org"])));
    }

    #[test]
    fn list_ids() {
        assert_eq!(list_id("https://easylist.to/easylist/easyprivacy.txt"), "easyprivacy");
    }

    /// Converts a real list for a look, or a compile by hand:
    /// `ORBIT_SHIELD_LIST=easylist.txt cargo test -p orbit-pathfinder dump -- --ignored`.
    #[test]
    #[ignore]
    fn dump() {
        let path = std::env::var("ORBIT_SHIELD_LIST").expect("ORBIT_SHIELD_LIST");
        let (net, hide) = convert(&std::fs::read_to_string(&path).unwrap());
        std::fs::write(format!("{path}.block.json"), net.to_string()).unwrap();
        std::fs::write(format!("{path}.hide.json"), hide.to_string()).unwrap();
    }
}
