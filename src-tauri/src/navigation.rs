//! Page-local wiki outlines. Never use the merged tag graph as a page's list.
use crate::catalog::Result;
use regex::Regex;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub display: String,
    pub kind: String,
    pub entries: Vec<Entry>,
    pub description: String,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Page {
    pub description: String,
    pub entries: Vec<Entry>,
}

pub fn kind(name: &str) -> &str {
    if name.starts_with("section:") || name.starts_with("outline:") {
        "section"
    } else if ["tag_groups", "pool_groups"].contains(&name)
        || [
            "tag_group:",
            "pool_group:",
            "list_of_",
            "help:",
            "howto:",
            "about:",
            "template:",
        ]
        .iter()
        .any(|p| name.starts_with(p))
    {
        "group"
    } else {
        "tag"
    }
}
fn readable(s: &str) -> String {
    static LINKS: std::sync::LazyLock<Regex> =
        std::sync::LazyLock::new(|| Regex::new(r"\[\[([^|\]]+)(?:\|([^\]]*))?\]\]").unwrap());
    let s = LINKS
        .replace_all(s, |m: &regex::Captures| {
            m.get(2).map(|v| v.as_str()).unwrap_or(&m[1]).to_string()
        })
        .replace('_', " ");
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
        .unwrap_or_default()
}
fn at<'a>(entries: &'a mut Vec<Entry>, path: &[usize]) -> &'a mut Vec<Entry> {
    if let Some((i, rest)) = path.split_first() {
        at(&mut entries[*i].entries, rest)
    } else {
        entries
    }
}
fn append(entries: &mut Vec<Entry>, parent: &[usize], entry: Entry) -> Vec<usize> {
    let siblings = at(entries, parent);
    let mut path = parent.to_vec();
    path.push(siblings.len());
    siblings.push(entry);
    path
}

pub fn outline(db: &Connection, page: &str) -> Result<Page> {
    let owner = page
        .strip_prefix("section:")
        .and_then(|s| s.split_once('#'))
        .map(|p| p.0);
    if let Some(owner) = owner {
        let full = outline(db, owner)?;
        fn find(entries: &[Entry], name: &str) -> Option<Entry> {
            entries.iter().find_map(|e| {
                if e.name == name {
                    Some(e.clone())
                } else {
                    find(&e.entries, name)
                }
            })
        }
        return Ok(find(&full.entries, page)
            .map(|e| Page {
                description: e.description,
                entries: e.entries,
            })
            .unwrap_or(Page {
                description: String::new(),
                entries: vec![],
            }));
    }
    let source: String = db.query_row(
        "SELECT coalesce((SELECT description FROM overlay.edits WHERE name=?1), (SELECT description FROM overlay.custom WHERE name=?1), (SELECT description FROM nodes WHERE name=?1), '')", [page], |r| r.get(0)).map_err(|e|e.to_string())?;
    static URL_LINKS: std::sync::LazyLock<Regex> = std::sync::LazyLock::new(|| {
        Regex::new(r#""([^"]+)":\[?(?:https?://[^/\s]+)?/wiki_pages/([^#?\]\s]+)(?:#[^\]\s]+)?\]?"#)
            .unwrap()
    });
    let source = URL_LINKS.replace_all(&source, |m: &regex::Captures| {
        format!(
            "[[{}|{}]]",
            crate::indexer::normalize(&urlencoding::decode(&m[2]).unwrap_or((&m[2]).into())),
            &m[1]
        )
    });
    let prefix = format!("section:{page}#");
    let mut stmt = db.prepare("SELECT n.name,n.display_name,p.name FROM nodes n LEFT JOIN edges e ON e.child_id=n.id LEFT JOIN nodes p ON p.id=e.parent_id WHERE n.name>=?1 AND n.name<?2 ORDER BY e.position").map_err(|e|e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![prefix, format!("section:{page}$")], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<String>>(2)?,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut sections = HashMap::new();
    let mut parents = HashMap::new();
    for row in rows {
        let (name, label, parent) = row.map_err(|e| e.to_string())?;
        sections.insert(name.clone(), label);
        if let Some(p) = parent {
            if p != page {
                parents.insert(name, p);
            }
        }
    }
    let mut sections: Vec<_> = sections.into_iter().collect();
    sections.sort_by_key(|(name, _)| {
        name.rsplit('-')
            .next()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(0)
    });
    let toc = Regex::new(r#"^\s*-\s+[\d.]+\s+\"[^\"]+\":#dtext-\S+\s*$"#).unwrap();
    let modern: bool = db
        .query_row(
            "SELECT value='headings-v2' FROM meta WHERE key='outline_format'",
            [],
            |r| r.get(0),
        )
        .unwrap_or(false);
    let heading = Regex::new(if modern {
        r"^(#{1,6})\s+(.*)$"
    } else {
        r"^(#{2,6})\s+(.*)$"
    })
    .unwrap();
    let bullet = Regex::new(r"^([ \t]*)(-|#+)\s+(.*)$").unwrap();
    let link = Regex::new(r"\[\[([^|\]]+)(?:\|([^\]]*))?\]\]").unwrap();
    let mut result = Page {
        description: String::new(),
        entries: vec![],
    };
    let mut current = vec![];
    let mut headings: HashMap<String, Vec<usize>> = HashMap::new();
    let mut bullets: Vec<(usize, Vec<usize>)> = vec![];
    let mut heading_stack: Vec<(usize, Vec<usize>)> = vec![];
    // Old releases flattened heading markers to ##. Their indexed section edges
    // retain the original levels; newer imports retain the markers as well.
    let explicit_levels = modern;
    for (line_number, line) in source.lines().enumerate() {
        if toc.is_match(line) {
            continue;
        }
        if let Some(h) = heading.captures(line) {
            let title = &h[2];
            let level = h[1].len();
            let linked = link
                .captures(title)
                .filter(|m| m.get(0).unwrap().as_str() == title);
            let (name, display, entry_kind) = if let Some(m) = linked {
                let name = m[1].to_string();
                let k = kind(&name).to_string();
                (
                    name,
                    readable(m.get(2).map(|s| s.as_str()).unwrap_or(&m[1])),
                    k,
                )
            } else {
                let found = sections
                    .iter()
                    .position(|(_, label)| label.eq_ignore_ascii_case(title));
                let name = found
                    .map(|i| sections.remove(i).0)
                    .unwrap_or_else(|| format!("outline:{page}:heading:{line_number}"));
                (name, readable(title), "section".to_string())
            };
            if name == page || name == "tag_groups" {
                continue;
            }
            while heading_stack.last().is_some_and(|(n, _)| *n >= level) {
                heading_stack.pop();
            }
            let parent = if entry_kind != "section" && !explicit_levels {
                // Linked headings also lost their levels in old catalogs. Only
                // consult their parent on this page, never their global children.
                let parent:Option<String> = db.query_row("SELECT p.name FROM edges e JOIN nodes p ON p.id=e.parent_id JOIN nodes n ON n.id=e.child_id WHERE n.name=?1 AND (p.name=?2 OR (p.name>=?3 AND p.name<?4)) ORDER BY (p.name=?2) DESC,e.position LIMIT 1",rusqlite::params![name,page,prefix,format!("section:{page}$")],|r|r.get(0)).optional().map_err(|e|e.to_string())?;
                parent
                    .and_then(|p| headings.get(&p).cloned())
                    .unwrap_or_default()
            } else if explicit_levels {
                heading_stack
                    .last()
                    .map(|(_, p)| p.clone())
                    .unwrap_or_default()
            } else {
                parents
                    .get(&name)
                    .and_then(|p| headings.get(p))
                    .cloned()
                    .unwrap_or_default()
            };
            current = append(
                &mut result.entries,
                &parent,
                Entry {
                    name: name.clone(),
                    display,
                    kind: entry_kind,
                    entries: vec![],
                    description: String::new(),
                    note: String::new(),
                },
            );
            headings.insert(name, current.clone());
            heading_stack.push((level, current.clone()));
            bullets.clear();
            continue;
        }
        let b = bullet.captures(line);
        let standalone = line.trim().starts_with("[[")
            && link.find(line).is_some_and(|m| {
                let tail = line[m.end()..].trim();
                tail.is_empty()
                    || tail.starts_with('(')
                    || tail.starts_with(',')
                    || tail.starts_with('/')
                    || tail.starts_with("or ")
                    || tail.starts_with("and ")
            });
        let table = line.trim().starts_with('|') && line.trim().ends_with('|');
        let (depth, content) = if let Some(ref b) = b {
            (
                b[1].replace('\t', "  ").len() / 2 + b[2].len() - 1,
                b[3].trim(),
            )
        } else if standalone || table {
            (0, line.trim())
        } else {
            if line.trim().starts_with("[See [[tag_groups") {
                continue;
            }
            let target = if current.is_empty() {
                &mut result.description
            } else {
                let (last, parent) = current.split_last().unwrap();
                &mut at(&mut result.entries, parent)[*last].description
            };
            target.push_str(line);
            target.push('\n');
            continue;
        };
        if content.is_empty() {
            continue;
        }
        while bullets.last().is_some_and(|(d, _)| *d >= depth) {
            bullets.pop();
        }
        let parent = bullets
            .last()
            .map(|(_, p)| p.clone())
            .unwrap_or_else(|| current.clone());
        let matches: Vec<_> = link.captures_iter(content).collect();
        let mut first = None;
        // Only listed links are navigation. Prose links remain in the description.
        if matches.is_empty() {
            first = Some(append(
                &mut result.entries,
                &parent,
                Entry {
                    name: format!("outline:{page}:list:{line_number}"),
                    display: readable(content),
                    kind: "section".into(),
                    entries: vec![],
                    description: String::new(),
                    note: String::new(),
                },
            ));
        } else {
            for m in &matches {
                let name = crate::indexer::normalize(&m[1]);
                if name == page || name == "tag_groups" {
                    continue;
                }
                let display = readable(m.get(2).map(|s| s.as_str()).unwrap_or(&m[1]));
                let k = if display.to_lowercase().starts_with("tag group:") {
                    "group"
                } else {
                    kind(&name)
                };
                let path = append(
                    &mut result.entries,
                    &parent,
                    Entry {
                        name: name.clone(),
                        display,
                        kind: k.into(),
                        entries: vec![],
                        description: String::new(),
                        note: if matches.len() == 1 {
                            content[m.get(0).unwrap().end()..].trim().to_string()
                        } else {
                            String::new()
                        },
                    },
                );
                if first.is_none() {
                    first = Some(path);
                }
            }
        }
        if let Some(path) = first {
            bullets.push((depth, path));
        }
    }
    fn prune(entries: &mut Vec<Entry>) {
        for e in entries.iter_mut() {
            prune(&mut e.entries);
            e.description = e.description.trim().into();
        }
        entries
            .retain(|e| e.kind != "section" || !e.entries.is_empty() || !e.description.is_empty());
    }
    prune(&mut result.entries);
    result.description = result.description.trim().into();
    // User-created links remain available without altering imported wiki lists.
    let mut custom = db
        .prepare("SELECT child FROM overlay.custom_edges WHERE parent=? ORDER BY rowid")
        .map_err(|e| e.to_string())?;
    for name in custom
        .query_map([page], |r| r.get::<_, String>(0))
        .map_err(|e| e.to_string())?
    {
        let name = name.map_err(|e| e.to_string())?;
        fn contains(es: &[Entry], n: &str) -> bool {
            es.iter().any(|e| e.name == n || contains(&e.entries, n))
        }
        if contains(&result.entries, &name) {
            continue;
        }
        let k: Option<String> = db
            .query_row(
                "SELECT kind FROM overlay.custom WHERE name=?",
                [&name],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        result.entries.push(Entry {
            display: readable(&name),
            kind: k.unwrap_or_else(|| kind(&name).into()),
            name,
            entries: vec![],
            description: String::new(),
            note: String::new(),
        });
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn database() -> Connection {
        let db = Connection::open(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("resources/codex-v1.sqlite"),
        )
        .unwrap();
        db.execute_batch("ATTACH ':memory:' AS overlay; CREATE TABLE overlay.edits(name,description); CREATE TABLE overlay.custom(name,kind,description); CREATE TABLE overlay.custom_edges(parent,child,kind);").unwrap();
        db
    }
    fn find<'a>(entries: &'a [Entry], name: &str) -> &'a Entry {
        fn seek<'a>(es: &'a [Entry], n: &str) -> Option<&'a Entry> {
            es.iter().find_map(|e| {
                if e.name == n {
                    Some(e)
                } else {
                    seek(&e.entries, n)
                }
            })
        }
        seek(entries, name).unwrap_or_else(|| panic!("Missing {name}"))
    }
    fn names(entries: &[Entry]) -> Vec<&str> {
        entries.iter().map(|e| e.name.as_str()).collect()
    }
    #[test]
    fn bundled_main_tree_is_page_local() {
        let db = database();
        let main = outline(&db, "tag_groups").unwrap();
        assert_eq!(
            main.entries
                .iter()
                .map(|e| e.display.as_str())
                .collect::<Vec<_>>(),
            vec![
                "Intro",
                "Visual characteristics",
                "Copyrights, artists, projects and media",
                "Metatags",
                "See also"
            ]
        );
        let body = find(&main.entries, "tag_group:body_parts");
        assert_eq!(
            names(&body.entries),
            vec![
                "tag_group:ass",
                "tag_group:breasts_tags",
                "tag_group:face_tags",
                "tag_group:hair",
                "tag_group:hands",
                "tag_group:feet",
                "tag_group:neck_and_neckwear",
                "on",
                "tag_group:posture",
                "tag_group:pussy",
                "tag_group:shoulders",
                "tag_group:skin_color",
                "tag_group:wings"
            ]
        );
        assert_eq!(
            names(&find(&main.entries, "tag_group:face_tags").entries),
            vec![
                "tag_group:ears_tags",
                "tag_group:eyes_tags",
                "eyebrows",
                "nose"
            ]
        );
        assert_eq!(
            names(&find(&main.entries, "tag_group:hair").entries),
            vec!["tag_group:hair_color", "tag_group:hair_styles"]
        );
        assert_eq!(find(&main.entries, "on").kind, "group");
        assert_eq!(find(&main.entries, "tag_group:metatags").kind, "group");
    }
    #[test]
    fn full_tag_wikis_and_nested_lists() {
        let db = database();
        let feet = outline(&db, "tag_group:feet").unwrap();
        let preview = find(&feet.entries, "toes");
        assert_eq!(preview.kind, "tag");
        assert!(!preview.entries.is_empty());
        let toes = outline(&db, "toes").unwrap();
        assert!(toes.description.contains("Digits of the"));
        assert_eq!(
            toes.entries
                .iter()
                .map(|e| e.display.as_str())
                .collect::<Vec<_>>(),
            vec!["Related tags", "See also"]
        );
        let related = &toes.entries[0];
        assert_eq!(
            related
                .entries
                .iter()
                .map(|e| e.display.as_str())
                .collect::<Vec<_>>(),
            vec!["Appearance", "Actions"]
        );
        for name in [
            "egyptian_toe",
            "long_toes",
            "toe_socks",
            "toe_sucking",
            "prehensile_toes",
            "prehensile_feet",
            "toenails",
            "dorsiflexion",
            "plantar_flexion",
        ] {
            find(&toes.entries, name);
        }
        assert_eq!(
            names(&find(&toes.entries, "toenails").entries),
            vec!["long_toenails", "toenail_polish", "sharp_toenails"]
        );
        let nails = outline(&db, "toenails").unwrap();
        assert!(nails.description.contains("noticeable"));
        assert_eq!(nails.entries[0].display, "See also");
        assert_eq!(names(&nails.entries[0].entries), vec!["fingernails"]);
        assert_eq!(
            outline(&db, "tag_group:ass").unwrap().entries[0].display,
            "Anatomy of the ass"
        );
    }
    #[test]
    fn outside_visual_and_all_main_categories_keep_list_links() {
        let db = database();
        let main = outline(&db, "tag_groups").unwrap();
        fn groups(es: &[Entry], out: &mut Vec<String>) {
            for e in es {
                if e.kind == "group" {
                    out.push(e.name.clone());
                }
                groups(&e.entries, out);
            }
        }
        let mut pages = vec![];
        groups(&main.entries, &mut pages);
        pages.extend(
            [
                "toes",
                "toenails",
                "ass",
                "hair",
                "feet",
                "pokemon",
                "hatsune_miku",
                "kyoto_animation",
            ]
            .map(String::from),
        );
        pages.sort();
        pages.dedup();
        let link = Regex::new(r"(?m)^[ \t]*-\s+\[\[([^|\]]+)").unwrap();
        for page in pages {
            let source: String = db
                .query_row("SELECT description FROM nodes WHERE name=?", [&page], |r| {
                    r.get(0)
                })
                .unwrap();
            let parsed = outline(&db, &page).unwrap();
            for m in link.captures_iter(&source) {
                if &m[1] != page && &m[1] != "tag_groups" {
                    find(&parsed.entries, &m[1]);
                }
            }
        }
        for page in [
            "tag_group:fighting_games",
            "list_of_official_mascots",
            "tag_group:metatags",
            "pool_groups",
            "list_of_meta-wikis",
        ] {
            assert!(!outline(&db, page).unwrap().entries.is_empty(), "{page}");
        }
        assert!(
            outline(&db, "list_of_pokemon_trainer_classes")
                .unwrap()
                .entries
                .len()
                > 100
        );
    }
    #[test]
    fn live_reference_import_preserves_source_hierarchy_and_full_lists() {
        let reference: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../tests/fixtures/wiki-reference.json")).unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let raw = Connection::open_in_memory().unwrap();
        raw.execute_batch("CREATE TABLE raw_wiki(title TEXT,wiki_id INTEGER,body TEXT,updated_at TEXT,other_names TEXT)").unwrap();
        for page in &reference {
            raw.execute(
                "INSERT INTO raw_wiki VALUES(?1,?2,?3,?4,'[]')",
                rusqlite::params![
                    page["title"].as_str(),
                    page["id"].as_i64(),
                    page["body"].as_str(),
                    page["updated_at"].as_str()
                ],
            )
            .unwrap();
        }
        let path = tmp.path().join("fresh.sqlite");
        crate::indexer::build(&raw, &path, || Ok(()), |_, _| {}).unwrap();
        let fresh = Connection::open(path).unwrap();
        fresh.execute_batch("ATTACH ':memory:' AS overlay; CREATE TABLE overlay.edits(name,description); CREATE TABLE overlay.custom(name,kind,description); CREATE TABLE overlay.custom_edges(parent,child,kind);").unwrap();
        let old = database();
        fn shape(es: &[Entry]) -> serde_json::Value {
            serde_json::json!(es
                .iter()
                .map(|e| serde_json::json!([
                    if e.kind == "section" {
                        "section"
                    } else {
                        &e.name
                    },
                    e.display,
                    e.kind,
                    shape(&e.entries)
                ]))
                .collect::<Vec<_>>())
        }
        for page in [
            "tag_groups",
            "toes",
            "toenails",
            "tag_group:ass",
            "tag_group:feet",
            "tag_group:hair",
            "tag_group:face_tags",
            "tag_group:fighting_games",
            "tag_group:metatags",
            "pool_groups",
        ] {
            assert_eq!(
                shape(&outline(&fresh, page).unwrap().entries),
                shape(&outline(&old, page).unwrap().entries),
                "{page}"
            );
        }
        assert_eq!(
            fresh
                .query_row("SELECT kind FROM nodes WHERE name='toes'", [], |r| r
                    .get::<_, String>(0))
                .unwrap(),
            "tag"
        );
    }
}
