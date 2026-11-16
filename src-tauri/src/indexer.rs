use crate::catalog::{counts, now, validate, App, Result};
use regex::Regex;
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    fs,
    path::Path,
    sync::{atomic::Ordering, Arc},
    time::{Duration, Instant},
};
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn normalize(s: &str) -> String {
    s.split('#')
        .next()
        .unwrap_or("")
        .trim()
        .to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join("_")
        .split('_')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("_")
}
fn index_title(n: &str) -> bool {
    n.starts_with("tag_group:")
        || n.starts_with("list_of_")
        || n.starts_with("pool_group:")
        || ["tag_groups", "pool_groups"].contains(&n)
}
fn persist(app: &App) -> Result<()> {
    let j = app.job.lock().map_err(err)?;
    let tmp = app.dir.join("update.tmp");
    fs::write(&tmp, serde_json::to_vec(&*j).map_err(err)?).map_err(err)?;
    fs::rename(tmp, app.dir.join("update.json")).map_err(err)
}
pub fn control(app: Arc<App>, action: &str) -> Result<Value> {
    if action == "pause" || action == "cancel" {
        if !app.running.load(Ordering::SeqCst) {
            let job = app.job.lock().map_err(err)?;
            if action == "pause"
                || !["paused", "interrupted", "error"]
                    .contains(&job["status"].as_str().unwrap_or(""))
            {
                return Ok(job.clone());
            }
        }
        app.control
            .store(if action == "pause" { 1 } else { 2 }, Ordering::SeqCst);
        if !app.running.load(Ordering::SeqCst) && action == "cancel" {
            let _ = fs::remove_dir_all(app.dir.join("staging"));
            app.job.lock().map_err(err)?["status"] = json!("cancelled");
            persist(&app)?;
        }
        return Ok(app.job.lock().map_err(err)?.clone());
    }
    if !["start", "resume", "retry"].contains(&action) {
        return Err("Invalid update action".into());
    }
    if app
        .running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Ok(app.job.lock().map_err(err)?.clone());
    }
    {
        let mut job = app.job.lock().map_err(err)?;
        if action != "start"
            && !["paused", "interrupted", "error"].contains(&job["status"].as_str().unwrap_or(""))
        {
            app.running.store(false, Ordering::SeqCst);
            return Err("No interrupted update to resume. Click Update Index to start.".into());
        }
        if action == "start" {
            let _ = fs::remove_dir_all(app.dir.join("staging"));
            *job = json!({"status":"fetch","after":0,"pages":0,"failures":[],"elapsed":0});
        }
        job["status"] = json!("fetch");
        job["started"] = json!(now());
    }
    app.control.store(0, Ordering::SeqCst);
    if let Err(error) = persist(&app) {
        app.running.store(false, Ordering::SeqCst);
        app.job.lock().map_err(err)?["status"] = json!("error");
        return Err(error);
    }
    let worker = app.clone();
    std::thread::spawn(move || {
        let started = Instant::now();
        let result = run(&worker, started);
        {
            let mut j = worker.job.lock().unwrap();
            j["elapsed"] = json!(j["elapsed"].as_u64().unwrap_or(0) + started.elapsed().as_secs());
            match result {
                Ok(()) => j["status"] = json!("done"),
                Err(e) => {
                    let c = worker.control.load(Ordering::SeqCst);
                    j["status"] = json!(if c == 1 {
                        "paused"
                    } else if c == 2 {
                        "cancelled"
                    } else {
                        "error"
                    });
                    j["message"] = json!(e.clone());
                    if c == 0 {
                        if let Some(f) = j["failures"].as_array_mut() {
                            f.push(json!({"at":now(),"message":e}));
                        }
                    }
                }
            }
        }
        if worker.control.load(Ordering::SeqCst) == 2 {
            let _ = fs::remove_dir_all(worker.dir.join("staging"));
        }
        let _ = persist(&worker);
        worker.running.store(false, Ordering::SeqCst);
    });
    Ok(app.job.lock().map_err(err)?.clone())
}
fn checkpoint(app: &App) -> Result<()> {
    match app.control.load(Ordering::SeqCst) {
        1 => Err("Update paused. Active index unchanged.".into()),
        2 => Err("Update cancelled. Active index unchanged.".into()),
        _ => Ok(()),
    }
}
fn wait(app: &App, seconds: u64) -> Result<()> {
    for _ in 0..seconds * 10 {
        checkpoint(app)?;
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}
fn run(app: &App, _started: Instant) -> Result<()> {
    let stage = app.dir.join("staging");
    fs::create_dir_all(&stage).map_err(err)?;
    let raw = Connection::open(stage.join("raw.sqlite")).map_err(err)?;
    raw.execute_batch("CREATE TABLE IF NOT EXISTS raw_wiki(title TEXT PRIMARY KEY,wiki_id INTEGER,body TEXT,updated_at TEXT,other_names TEXT);CREATE TABLE IF NOT EXISTS progress(id INTEGER);INSERT INTO progress SELECT 0 WHERE NOT EXISTS(SELECT 1 FROM progress);").map_err(err)?;
    let endpoint = "https://danbooru.donmai.us/wiki_pages.json".to_string();
    #[cfg(test)]
    let endpoint = app.endpoint.lock().unwrap().clone().unwrap_or(endpoint);
    let client = reqwest::blocking::Client::builder()
        .user_agent("DanbooruTagIndexer/3.0 (manual offline encyclopedia update)")
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(err)?;
    loop {
        checkpoint(app)?;
        let after: i64 = raw
            .query_row("SELECT id FROM progress", [], |r| r.get(0))
            .map_err(err)?;
        let mut batch = None;
        for attempt in 0..6 {
            checkpoint(app)?;
            let response = client
                .get(&endpoint)
                .query(&[
                    ("limit", "1000".to_string()),
                    ("page", format!("a{after}")),
                    (
                        "only",
                        "id,title,body,updated_at,is_deleted,other_names".to_string(),
                    ),
                ])
                .send();
            match response {
                Ok(r) if r.status().is_success() => {
                    batch = Some(r.json::<Vec<Value>>().map_err(err)?);
                    break;
                }
                Ok(r) if r.status().as_u16() == 429 || r.status().is_server_error() => {
                    if attempt == 5 {
                        return Err(format!("Danbooru returned {}", r.status()));
                    }
                }
                Ok(r) => return Err(format!("Danbooru returned {}", r.status())),
                Err(e) => {
                    if attempt == 5 {
                        return Err(e.to_string());
                    }
                }
            }
            wait(app, 2u64.pow(attempt + 1).min(30))?;
        }
        let batch = batch.ok_or("No response")?;
        if batch.is_empty() {
            break;
        }
        let mut max = after;
        raw.execute_batch("BEGIN").map_err(err)?;
        for row in &batch {
            max = max.max(row["id"].as_i64().unwrap_or(0));
            if row["is_deleted"] == true {
                continue;
            }
            if let Some(title) = row["title"].as_str() {
                raw.execute(
                    "INSERT OR REPLACE INTO raw_wiki VALUES(?1,?2,?3,?4,?5)",
                    params![
                        title,
                        row["id"].as_i64(),
                        row["body"].as_str().unwrap_or(""),
                        row["updated_at"].as_str(),
                        row["other_names"].to_string()
                    ],
                )
                .map_err(err)?;
            }
        }
        if max <= after {
            return Err("Danbooru pagination did not advance".into());
        }
        raw.execute("UPDATE progress SET id=?", [max])
            .map_err(err)?;
        raw.execute_batch("COMMIT").map_err(err)?;
        {
            let mut j = app.job.lock().map_err(err)?;
            j["after"] = json!(max);
            j["pages"] = json!(j["pages"].as_u64().unwrap_or(0) + batch.len() as u64);
            j["message"] = json!(format!("Downloaded wiki pages through ID {max}"));
        }
        persist(app)?;
        wait(app, 1)?;
    }
    app.job.lock().map_err(err)?["status"] = json!("build");
    persist(app)?;
    let next = stage.join("next.sqlite");
    let _ = fs::remove_file(&next);
    build(
        &raw,
        &next,
        || checkpoint(app),
        |pages, nodes| {
            let stats = crate::catalog::ro(&next).and_then(|db| counts(&db)).ok();
            let mut j = app.job.lock().unwrap();
            if let Some(stats) = stats {
                j["tags"] = json!(stats[0]);
                j["categories"] = json!(stats[1]);
                j["relationships"] = json!(stats[2]);
            }
            j["message"] = json!(format!(
                "Building hierarchy: {pages} pages, {nodes} entries"
            ));
        },
    )?;
    checkpoint(app)?;
    app.job.lock().map_err(err)?["status"] = json!("validate");
    validate(&next)?;
    {
        let _gate = app.gate.lock().map_err(err)?;
        checkpoint(app)?;
        app.activate(&next, "Manual update", true, false)?;
        let warning = app.retention().err();
        let mut job = app.job.lock().map_err(err)?;
        job["message"]=json!(warning.map(|e|format!("Index activated. Retention cleanup will need attention: {e}")).unwrap_or_else(||"New index validated and activated. Previous versions retained according to your settings.".to_string()));
    }
    drop(raw);
    let _ = fs::remove_dir_all(stage);
    Ok(())
}
#[derive(Clone)]
struct Edge {
    parent: String,
    child: String,
    kind: String,
}
struct Parsed {
    rich: String,
    prose: String,
    sections: Vec<(String, String)>,
    edges: Vec<Edge>,
    bullets: usize,
    linked: usize,
    chars: usize,
}
struct Parser {
    link: Regex,
    heading: Regex,
    bullet: Regex,
    expand: Regex,
    media: Regex,
    format: Regex,
    slug: Regex,
    external: Regex,
}
impl Parser {
    fn new() -> Self {
        Self {
            link: Regex::new(r"\[\[([^\[\]|]+)(?:\|([^\[\]]*))?\]\]").unwrap(),
            heading: Regex::new(r"(?i)^h([1-6])(?:#([A-Za-z0-9_-]+))?\.\s*(.*)$").unwrap(),
            bullet: Regex::new(r"^(\*+)\s+(.*)$").unwrap(),
            expand: Regex::new(r"(?i)\[/?expand(?:=[^\]]*)?\]").unwrap(),
            media: Regex::new(r"!(?:post|asset)\s*#\d+:?").unwrap(),
            format: Regex::new(r"\[/?[a-z][a-z0-9]*(?:=[^\]]*)?\]").unwrap(),
            slug: Regex::new(r"[^a-z0-9]+").unwrap(),
            external: Regex::new(r#""([^"]+)":(?:https?://[^/\s]+)?/wiki_pages/([^#?\s]+)"#)
                .unwrap(),
        }
    }
    fn links(&self, s: &str) -> Vec<String> {
        let mut seen = HashSet::new();
        self.link
            .captures_iter(s)
            .map(|c| normalize(&c[1]))
            .filter(|s| !s.is_empty() && seen.insert(s.clone()))
            .collect()
    }
    fn clean(&self, s: &str) -> String {
        // Canonicalize links before stripping DText formatting: [[body]] must
        // never be mistaken for a single-bracket formatting token.
        let linked = self.link.replace_all(s, |c: &regex::Captures| {
            let name = normalize(&c[1]);
            let label = c
                .get(2)
                .map(|m| m.as_str().to_string())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| name.replace('_', " "));
            format!("[[{name}|{label}]]")
        });
        self.format
            .replace_all(&self.media.replace_all(&linked, ""), "")
            .trim()
            .to_string()
    }

    fn parse(&self, name: &str, body: &str) -> Parsed {
        let source = self.expand.replace_all(body, "");
        let source = self.external.replace_all(&source, |c: &regex::Captures| {
            format!(
                "[[{}|{}]]",
                urlencoding::decode(&c[2]).unwrap_or((&c[2]).into()),
                &c[1]
            )
        });
        let mut p = Parsed {
            rich: String::new(),
            prose: String::new(),
            sections: vec![],
            edges: vec![],
            bullets: 0,
            linked: 0,
            chars: 0,
        };
        let mut stack = vec![(name.to_string(), 0, "child".to_string())];
        let mut lists: Vec<(usize, String)> = vec![];
        let mut heading_index = 0;
        for line in source.lines().map(str::trim) {
            if let Some(h) = self.heading.captures(line) {
                let level = h[1].parse::<u8>().unwrap();
                while stack.len() > 1 && stack.last().unwrap().1 >= level {
                    stack.pop();
                }
                let (parent, _, inherited) = stack.last().unwrap().clone();
                let title = &h[3];
                let lower = title.to_lowercase();
                let mut mode = if lower.starts_with("see also") {
                    "xref"
                } else if lower.starts_with("related") || lower.contains("similar tags") {
                    "related"
                } else {
                    &inherited
                }
                .to_string();
                if name == "tag_groups" && mode == "xref" {
                    mode = "child".into()
                }
                let links = self.links(title);
                let key = if links.len() == 1 && self.link.replace_all(title, "").trim().is_empty()
                {
                    links[0].clone()
                } else {
                    let display = self
                        .link
                        .replace_all(title, |c: &regex::Captures| {
                            c.get(2).map(|m| m.as_str()).unwrap_or(&c[1]).to_string()
                        })
                        .to_string();
                    let slug = h.get(2).map(|m| m.as_str().to_string()).unwrap_or_else(|| {
                        self.slug
                            .replace_all(&display.to_lowercase(), "-")
                            .trim_matches('-')
                            .chars()
                            .take(60)
                            .collect()
                    });
                    let key = format!("section:{name}#{slug}-{heading_index}");
                    heading_index += 1;
                    p.sections.push((key.clone(), display));
                    key
                };
                p.edges.push(Edge {
                    parent,
                    child: key.clone(),
                    kind: mode.clone(),
                });
                stack.push((key, level, mode));
                lists.clear();
                p.rich += &format!("## {}\n", self.clean(title));
                continue;
            }
            if let Some(b) = self.bullet.captures(line) {
                p.bullets += 1;
                let stars = b[1].len();
                let content = &b[2];
                let mut links = self.links(content);
                if !links.is_empty() {
                    p.linked += 1
                } else {
                    let clean = self.clean(content);
                    if clean.len() >= 2 && clean.len() <= 60 && !clean.contains(['.', '!', '?']) {
                        links.push(normalize(&clean));
                    }
                }
                while lists.last().is_some_and(|x| x.0 >= stars) {
                    lists.pop();
                }
                let parent = lists
                    .last()
                    .map(|x| x.1.clone())
                    .unwrap_or_else(|| stack.last().unwrap().0.clone());
                let mode = stack.last().unwrap().2.clone();
                for child in &links {
                    p.edges.push(Edge {
                        parent: parent.clone(),
                        child: child.clone(),
                        kind: mode.clone(),
                    });
                }
                if let Some(first) = links.first() {
                    lists.push((stars, first.clone()))
                }
                p.rich += &format!(
                    "{}- {}\n",
                    "  ".repeat(stars.saturating_sub(1)),
                    self.clean(content)
                );
                continue;
            }
            if line.starts_with('|') && line.ends_with('|') {
                for child in self.links(line) {
                    p.edges.push(Edge {
                        parent: stack.last().unwrap().0.clone(),
                        child,
                        kind: stack.last().unwrap().2.clone(),
                    });
                }
            } else {
                p.chars += self.clean(line).encode_utf16().count();
                p.prose += &format!("{}\n", self.clean(line));
                for child in self.links(line) {
                    p.edges.push(Edge {
                        parent: name.to_string(),
                        child,
                        kind: "xref".into(),
                    });
                }
            }
            p.rich += &format!("{}\n", self.clean(line));
        }
        p
    }
}
const SCHEMA:&str="CREATE TABLE nodes(id INTEGER PRIMARY KEY,name TEXT UNIQUE NOT NULL,display_name TEXT NOT NULL,kind TEXT NOT NULL,description TEXT NOT NULL DEFAULT '',source_url TEXT,source_wiki_id INTEGER,source_updated_at TEXT,other_names TEXT NOT NULL DEFAULT '[]',coverage TEXT NOT NULL DEFAULT 'full',origin TEXT NOT NULL DEFAULT 'danbooru',links_text TEXT NOT NULL DEFAULT '',child_count INTEGER NOT NULL DEFAULT 0);CREATE TABLE edges(id INTEGER PRIMARY KEY,parent_id INTEGER NOT NULL,child_id INTEGER NOT NULL,edge_type TEXT NOT NULL,position INTEGER NOT NULL,note TEXT NOT NULL DEFAULT '',UNIQUE(parent_id,child_id,edge_type));CREATE INDEX edges_parent ON edges(parent_id,edge_type,position);CREATE INDEX edges_child ON edges(child_id,edge_type);CREATE TABLE meta(key TEXT PRIMARY KEY,value TEXT NOT NULL);";
fn ensure(
    db: &Connection,
    raw: &Connection,
    parser: &Parser,
    ids: &mut HashMap<String, i64>,
    name: &str,
    display: Option<&str>,
) -> Result<i64> {
    if let Some(id) = ids.get(name) {
        return Ok(*id);
    }
    let rec: Option<(i64, String, Option<String>, String)> = raw
        .query_row(
            "SELECT wiki_id,body,updated_at,other_names FROM raw_wiki WHERE title=?",
            [name],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
        )
        .optional()
        .map_err(err)?;
    let section = name.starts_with("section:");
    let kind = if section {
        "section"
    } else if index_title(name) {
        "group"
    } else {
        "tag"
    };
    let description = rec
        .as_ref()
        .map(|r| parser.parse(name, &r.1).rich)
        .unwrap_or_default();
    let url = if section {
        format!(
            "https://danbooru.donmai.us/wiki_pages/{}",
            urlencoding::encode(
                name.trim_start_matches("section:")
                    .split('#')
                    .next()
                    .unwrap_or(name)
            )
        )
    } else {
        format!(
            "https://danbooru.donmai.us/wiki_pages/{}",
            urlencoding::encode(name)
        )
    };
    db.execute("INSERT INTO nodes(name,display_name,kind,description,source_url,source_wiki_id,source_updated_at,other_names,coverage) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![name,display.map(str::to_string).unwrap_or_else(||name.replace('_'," ")),kind,description,url,rec.as_ref().map(|r|r.0),rec.as_ref().and_then(|r|r.2.clone()),rec.as_ref().map(|r|r.3.clone()).unwrap_or("[]".into()),if section||rec.is_some(){"full"}else{"unindexed"}]).map_err(err)?;
    let id = db.last_insert_rowid();
    ids.insert(name.to_string(), id);
    Ok(id)
}
pub fn build(
    raw: &Connection,
    path: &Path,
    check: impl Fn() -> Result<()>,
    progress: impl Fn(usize, usize),
) -> Result<()> {
    let db = Connection::open(path).map_err(err)?;
    db.execute_batch(SCHEMA).map_err(err)?;
    let parser = Parser::new();
    let mut ids = HashMap::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::from(["tag_groups".to_string()]);
    let mut positions: HashMap<(i64, String), i64> = HashMap::new();
    db.execute_batch("BEGIN").map_err(err)?;
    while let Some(name) = queue.pop_front() {
        if name.starts_with("section:") || !visited.insert(name.clone()) {
            continue;
        }
        check()?;
        let id = ensure(&db, raw, &parser, &mut ids, &name, None)?;
        let body: Option<String> = raw
            .query_row("SELECT body FROM raw_wiki WHERE title=?", [&name], |r| {
                r.get(0)
            })
            .optional()
            .map_err(err)?;
        let Some(body) = body else { continue };
        let p = parser.parse(&name, &body);
        let article = p.chars > 700 && p.chars > p.bullets * 40;
        let expand = index_title(&name)
            || (!article && p.bullets >= 8 && p.linked as f64 / p.bullets as f64 >= 0.45);
        if expand {
            db.execute("UPDATE nodes SET kind='group' WHERE id=?", [id])
                .map_err(err)?;
        }
        // Section identities and their children must survive on descriptive articles too.
        for (key, display) in &p.sections {
            if expand {
                ensure(&db, raw, &parser, &mut ids, key, Some(display))?;
            }
        }
        for edge in p.edges {
            let section = edge.child.starts_with("section:");
            if !expand && (edge.kind == "child" || section) {
                continue;
            }
            let child = ensure(&db, raw, &parser, &mut ids, &edge.child, None)?;
            let parent = if expand {
                match ids.get(&edge.parent) {
                    Some(p) => *p,
                    None => ensure(&db, raw, &parser, &mut ids, &edge.parent, None)?,
                }
            } else {
                id
            };
            if parent == child {
                continue;
            }
            let pos = positions.entry((parent, edge.kind.clone())).or_default();
            db.execute("INSERT OR IGNORE INTO edges(parent_id,child_id,edge_type,position) VALUES(?1,?2,?3,?4)",params![parent,child,edge.kind,*pos]).map_err(err)?;
            *pos += 1;
            if expand && edge.kind == "child" && !section && !visited.contains(&edge.child) {
                queue.push_back(edge.child)
            }
        }
        if visited.len() % 250 == 0 {
            db.execute_batch("COMMIT;BEGIN").map_err(err)?;
            progress(visited.len(), ids.len());
        }
    }
    db.execute_batch("COMMIT;UPDATE nodes SET kind='group' WHERE kind='tag' AND id IN(SELECT parent_id FROM edges WHERE edge_type='child');UPDATE nodes SET child_count=(SELECT count(*) FROM edges WHERE parent_id=nodes.id AND edge_type='child');UPDATE nodes SET links_text=coalesce((SELECT group_concat(n.name,' ') FROM edges e JOIN nodes n ON n.id=e.child_id WHERE e.parent_id=nodes.id),'');CREATE VIRTUAL TABLE nodes_fts USING fts5(name,display_name,description,links_text,other_names,content='nodes',content_rowid='id',tokenize='unicode61 remove_diacritics 2');INSERT INTO nodes_fts(nodes_fts) VALUES('rebuild');").map_err(err)?;
    check()?;
    let c = counts(&db)?;
    for (k, v) in [
        ("schema", "1".to_string()),
        ("built_at", now()),
        (
            "source",
            "https://danbooru.donmai.us/wiki_pages/tag_groups".into(),
        ),
        (
            "stats",
            json!({"tags":c[0],"categories":c[1],"relationships":c[2]}).to_string(),
        ),
    ] {
        db.execute("INSERT INTO meta VALUES(?1,?2)", [k, &v])
            .map_err(err)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parser_separates_related_and_cycles() {
        let p=Parser::new().parse("tag_group:test","h2. Parts\n* [[feet]]\n** [[toes]]\nh2. Related tags\n* [[hands]]\nh2. See also\n* [[tag_group:test]]");
        assert!(p
            .edges
            .iter()
            .any(|e| e.parent == "feet" && e.child == "toes" && e.kind == "child"));
        assert!(p
            .edges
            .iter()
            .any(|e| e.child == "hands" && e.kind == "related"));
        assert!(p
            .edges
            .iter()
            .any(|e| e.child == "tag_group:test" && e.kind == "xref"));
    }
    #[test]
    fn root_see_also_structural() {
        let p = Parser::new().parse("tag_groups", "h2. See also\n* [[pool_groups]]");
        assert!(p.edges.iter().all(|e| e.kind == "child"));
    }
    #[test]
    fn graph_handles_deep_multiple_parents() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = Connection::open_in_memory().unwrap();
        raw.execute_batch("CREATE TABLE raw_wiki(title TEXT,wiki_id INTEGER,body TEXT,updated_at TEXT,other_names TEXT)").unwrap();
        for i in 0..40 {
            let name = if i == 0 {
                "tag_groups".into()
            } else {
                format!("tag_group:{i}")
            };
            let body = format!(
                "* [[tag_group:{}]]\n* [[shared]]",
                if i == 39 { 1 } else { i + 1 }
            );
            raw.execute(
                "INSERT INTO raw_wiki VALUES(?1,?2,?3,NULL,'[]')",
                params![name, i, body],
            )
            .unwrap();
        }
        build(&raw, &tmp.path().join("db"), || Ok(()), |_, _| {}).unwrap();
        let db = Connection::open(tmp.path().join("db")).unwrap();
        let n:i64=db.query_row("SELECT count(*) FROM edges e JOIN nodes n ON n.id=e.child_id WHERE n.name='shared'",[],|r|r.get(0)).unwrap();
        assert_eq!(n, 40);
    }
}
#[cfg(test)]
mod update_tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn explicit_update_fetches_builds_validates_and_activates() {
        let tmp = tempfile::tempdir().unwrap();
        let raw = Connection::open_in_memory().unwrap();
        raw.execute_batch("CREATE TABLE raw_wiki(title TEXT,wiki_id INTEGER,body TEXT,updated_at TEXT,other_names TEXT)").unwrap();
        let mut body=String::from("h2. Visual characteristics\nh2. Copyrights, artists, projects and media\nh2. Metatags\nh2. See also\n* [[pool_groups]]\n* [[list_of_meta-wikis]]\n");
        let mut batch = vec![
            json!({"id":2,"title":"pool_groups","body":"","other_names":[]}),
            json!({"id":3,"title":"list_of_meta-wikis","body":"","other_names":[]}),
        ];
        for i in 0..1100 {
            let name = format!("test_{i}");
            body += &format!("* [[{name}]]\n");
            batch.push(json!({"id":i+4,"title":name,"body":"Description","other_names":[]}));
        }
        batch.push(json!({"id":1,"title":"tag_groups","body":body,"other_names":[]}));
        batch.sort_by_key(|row| row["id"].as_i64().unwrap());
        for row in &batch {
            raw.execute(
                "INSERT INTO raw_wiki VALUES(?1,?2,?3,NULL,'[]')",
                params![
                    row["title"].as_str(),
                    row["id"].as_i64(),
                    row["body"].as_str()
                ],
            )
            .unwrap();
        }
        let seed = tmp.path().join("seed.sqlite");
        build(&raw, &seed, || Ok(()), |_, _| {}).unwrap();
        let app = Arc::new(App::new(tmp.path().join("app"), seed).unwrap());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        *app.endpoint.lock().unwrap() = Some(format!(
            "http://{}/wiki_pages.json",
            listener.local_addr().unwrap()
        ));
        let pause_app = app.clone();
        let server = std::thread::spawn(move || {
            // Short nonempty pages are not proof of end-of-pagination.
            for payload in [
                serde_json::to_string(&batch[..500]).unwrap(),
                serde_json::to_string(&batch[500..]).unwrap(),
                "[]".to_string(),
            ]
            .into_iter()
            .enumerate()
            {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = [0; 4096];
                stream.read(&mut req).unwrap();
                let (i, payload) = payload;
                if i == 0 {
                    control(pause_app.clone(), "pause").unwrap();
                }
                let response=format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",payload.len(),payload);
                stream.write_all(response.as_bytes()).unwrap();
            }
        });
        assert!(control(app.clone(), "resume").is_err());
        control(app.clone(), "start").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.running.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(app.job.lock().unwrap()["status"], "paused");
        assert_eq!(
            app.meta().unwrap()["snapshots"].as_array().unwrap().len(),
            1
        );
        control(app.clone(), "resume").unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        while app.running.load(Ordering::SeqCst) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(app.job.lock().unwrap()["status"], "done");
        assert_eq!(
            app.meta().unwrap()["snapshots"].as_array().unwrap().len(),
            2
        );
        server.join().unwrap();
        let before = app.meta().unwrap();
        app.control.store(1, Ordering::SeqCst);
        assert!(run(&app, Instant::now()).is_err());
        assert_eq!(app.meta().unwrap(), before);
        app.control.store(2, Ordering::SeqCst);
        assert!(run(&app, Instant::now()).is_err());
        assert_eq!(app.meta().unwrap(), before);
    }
}
#[cfg(test)]
mod real_data_tests {
    use super::*;
    #[test]
    #[ignore = "requires a complete development-time raw_wiki SQLite dump"]
    fn rebuild_real_dump() {
        let input = std::env::var("TAG_CODEX_RAW_INDEX").expect("TAG_CODEX_RAW_INDEX");
        let output = std::env::var("TAG_CODEX_INDEX_OUTPUT").expect("TAG_CODEX_INDEX_OUTPUT");
        let raw = Connection::open(input).unwrap();
        let path = Path::new(&output);
        assert!(!path.exists());
        build(
            &raw,
            path,
            || Ok(()),
            |pages, nodes| {
                if pages % 10000 == 0 {
                    println!("{pages} pages / {nodes} nodes");
                }
            },
        )
        .unwrap();
        validate(path).unwrap();
        let db = crate::catalog::ro(path).unwrap();
        println!("Validated counts: {:?}", counts(&db).unwrap());
    }
}
#[cfg(test)]
mod text_tests {
    use super::*;
    #[test]
    fn collapsed_sections_and_prose_links_are_preserved() {
        let p = Parser::new().parse("tag_group:test", "[expand=Details]\nh2. Nested\n* [[hidden_tag]]\n[/expand]\nSee [[other_tag]] for details.");
        assert!(p.rich.contains("hidden_tag"));
        assert!(p
            .edges
            .iter()
            .any(|e| e.child == "hidden_tag" && e.kind == "child"));
        assert!(p
            .edges
            .iter()
            .any(|e| e.parent == "tag_group:test" && e.child == "other_tag" && e.kind == "xref"));
        assert!(!p.rich.contains("[expand"));
    }
    #[test]
    fn canonical_links_survive_formatting() {
        let p = Parser::new();
        assert_eq!(
            p.clean("[b]See[/b] [[Brown Hair]] and [[feet|Feet]]."),
            "See [[brown_hair|brown hair]] and [[feet|Feet]]."
        );
    }
}
