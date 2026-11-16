use rusqlite::{params, Connection, OpenFlags, OptionalExtension};
use serde_json::{json, Value};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{atomic::AtomicU8, Mutex},
};
pub type Result<T> = std::result::Result<T, String>;
fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}
pub fn now() -> String {
    chrono::Utc::now().to_rfc3339()
}
pub fn ro(p: &Path) -> Result<Connection> {
    Connection::open_with_flags(p, OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(err)
}
pub fn text<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub struct App {
    _lock: fs::File,
    #[cfg(test)]
    pub endpoint: Mutex<Option<String>>,
    pub dir: PathBuf,
    pub gate: Mutex<()>,
    pub job: Mutex<Value>,
    pub control: AtomicU8,
    pub running: std::sync::atomic::AtomicBool,
}
impl App {
    pub fn new(dir: PathBuf, seed: PathBuf) -> Result<Self> {
        fs::create_dir_all(dir.join("versions")).map_err(err)?;
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(dir.join("app.lock"))
            .map_err(err)?;
        lock.try_lock().map_err(|_| {
            "Danbooru Tag Indexer is already running for this data directory".to_string()
        })?;
        let registry = Connection::open(dir.join("catalog.sqlite")).map_err(err)?;
        registry.execute_batch("PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS snapshots(version INTEGER PRIMARY KEY,label TEXT,created TEXT,protected INTEGER,current INTEGER,file TEXT UNIQUE); CREATE TABLE IF NOT EXISTS custom(name TEXT PRIMARY KEY,display TEXT,kind TEXT,description TEXT); CREATE TABLE IF NOT EXISTS edits(name TEXT PRIMARY KEY,description TEXT); CREATE TABLE IF NOT EXISTS custom_edges(parent TEXT,child TEXT,kind TEXT,PRIMARY KEY(parent,child,kind)); PRAGMA user_version=1;").map_err(err)?;
        let count: i64 = registry
            .query_row("SELECT count(*) FROM snapshots", [], |r| r.get(0))
            .map_err(err)?;
        if count == 0 {
            validate(&seed)?;
            fs::copy(seed, dir.join("versions/v1.sqlite")).map_err(err)?;
            registry
                .execute(
                    "INSERT INTO snapshots VALUES(1,'Version 1',?1,0,1,'v1.sqlite')",
                    [now()],
                )
                .map_err(err)?;
        }
        let mut job: Value = fs::read(dir.join("update.json"))
            .ok()
            .and_then(|s| serde_json::from_slice(&s).ok())
            .unwrap_or(json!({"status":"idle","after":0,"pages":0,"failures":[],"elapsed":0}));
        if ["fetch", "build", "validate"].contains(&text(&job, "status")) {
            job["status"] = json!("interrupted");
        }
        Ok(Self {
            _lock: lock,
            #[cfg(test)]
            endpoint: Mutex::new(None),
            dir,
            gate: Mutex::new(()),
            job: Mutex::new(job),
            control: AtomicU8::new(0),
            running: std::sync::atomic::AtomicBool::new(false),
        })
    }
    pub fn registry(&self) -> Result<Connection> {
        Connection::open(self.dir.join("catalog.sqlite")).map_err(err)
    }
    pub fn version(&self, v: i64) -> Result<(Connection, i64)> {
        let r = self.registry()?;
        let (id, file): (i64, String) = if v == 0 {
            r.query_row(
                "SELECT version,file FROM snapshots WHERE current=1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        } else {
            r.query_row(
                "SELECT version,file FROM snapshots WHERE version=?",
                [v],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
        }
        .map_err(err)?;
        Ok((ro(&self.dir.join("versions").join(file))?, id))
    }
    fn attach(&self, db: &Connection) -> Result<()> {
        db.execute(
            "ATTACH DATABASE ? AS overlay",
            [self.dir.join("catalog.sqlite").to_string_lossy().as_ref()],
        )
        .map_err(err)?;
        Ok(())
    }
    pub fn dispatch(self: &std::sync::Arc<Self>, op: &str, a: &Value) -> Result<Value> {
        if op == "update" {
            return crate::indexer::control(self.clone(), text(a, "action"));
        }
        if op == "job" {
            let mut job = self.job.lock().map_err(err)?.clone();
            if ["fetch", "build", "validate"].contains(&text(&job, "status")) {
                if let Ok(start) = chrono::DateTime::parse_from_rfc3339(text(&job, "started")) {
                    let delta = (chrono::Utc::now() - start.with_timezone(&chrono::Utc))
                        .num_seconds()
                        .max(0) as u64;
                    job["elapsed"] = json!(job["elapsed"].as_u64().unwrap_or(0) + delta);
                }
            }
            return Ok(job);
        }
        let _lock = self.gate.lock().map_err(err)?;
        let v = a["version"].as_i64().unwrap_or(0);
        match op {
            "meta" => self.meta(),
            "children" => self.children(text(a, "name"), v),
            "node" => self.node(text(a, "name"), v),
            "search" => self.search(text(a, "query"), text(a, "kind"), v),
            "edit" => {
                let r = self.registry()?;
                r.execute("INSERT INTO edits VALUES(?1,?2) ON CONFLICT(name) DO UPDATE SET description=excluded.description",params![text(a,"name"),text(a,"description")]).map_err(err)?;
                Ok(json!(true))
            }
            "revert" => {
                self.registry()?
                    .execute("DELETE FROM edits WHERE name=?", [text(a, "name")])
                    .map_err(err)?;
                Ok(json!(true))
            }
            "create" => self.create(a),
            "edge" => self.edge(a),
            "deleteCustom" => self.delete_custom(text(a, "name")),
            "snapshot" => self.snapshot(a),
            "compare" => self.compare(
                a["left"].as_i64().unwrap_or(0),
                a["right"].as_i64().unwrap_or(0),
            ),
            "export" => self.export(a),
            "import" => self.import(a),
            _ => Err("Unknown operation".into()),
        }
    }
    pub fn meta(&self) -> Result<Value> {
        let r = self.registry()?;
        let mut stmt=r.prepare("SELECT version,label,created,protected,current,file FROM snapshots ORDER BY version DESC").map_err(err)?;
        let mut out = Vec::new();
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(3)?,
                    row.get::<_, bool>(4)?,
                    row.get::<_, String>(5)?,
                ))
            })
            .map_err(err)?;
        for row in rows {
            let (id, label, created, protected, current, file) = row.map_err(err)?;
            let p = self.dir.join("versions").join(file);
            let db = ro(&p)?;
            let stats = counts(&db)?;
            out.push(json!({"version":id,"label":label,"created":created,"protected":protected,"current":current,"tags":stats[0],"categories":stats[1],"relationships":stats[2],"bytes":fs::metadata(p).map_err(err)?.len()}));
        }
        Ok(json!({"snapshots":out}))
    }
    fn children(&self, name: &str, v: i64) -> Result<Value> {
        let (db, _) = self.version(v)?;
        self.attach(&db)?;
        let name = if name.is_empty() { "tag_groups" } else { name };
        let mut stmt=db.prepare("SELECT n.name,n.display_name,n.kind,n.coverage,n.child_count FROM edges e JOIN nodes n ON n.id=e.child_id JOIN nodes p ON p.id=e.parent_id WHERE p.name=? AND e.edge_type='child' ORDER BY e.position").map_err(err)?;
        let mut out = stmt
            .query_map([name], summary)
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        self.enrich_summaries(&db, &mut out)?;
        self.add_custom_links(&db, name, "child", false, &mut out)?;
        Ok(json!(out))
    }
    fn enrich_summaries(&self, db: &Connection, out: &mut [Value]) -> Result<()> {
        let mut stmt = db.prepare("SELECT parent,count(*) FROM overlay.custom_edges WHERE kind='child' GROUP BY parent").map_err(err)?;
        let extras = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(err)?
            .collect::<std::result::Result<std::collections::HashMap<_, _>, _>>()
            .map_err(err)?;
        for node in out {
            node["children"] = json!(
                node["children"].as_i64().unwrap_or(0)
                    + extras.get(text(node, "name")).copied().unwrap_or(0)
            );
            let custom: bool = db
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM overlay.custom WHERE name=?)",
                    [text(node, "name")],
                    |r| r.get(0),
                )
                .map_err(err)?;
            if custom {
                node["custom"] = json!(true);
            }
        }
        Ok(())
    }
    fn add_custom_links(
        &self,
        db: &Connection,
        name: &str,
        kind: &str,
        reverse: bool,
        out: &mut Vec<Value>,
    ) -> Result<()> {
        let sql = if reverse {
            "SELECT parent FROM overlay.custom_edges WHERE child=? AND kind=?"
        } else {
            "SELECT child FROM overlay.custom_edges WHERE parent=? AND kind=?"
        };
        let mut stmt = db.prepare(sql).map_err(err)?;
        let names = stmt
            .query_map([name, kind], |r| r.get::<_, String>(0))
            .map_err(err)?;
        for n in names {
            let n = n.map_err(err)?;
            if out.iter().any(|x| text(x, "name") == n) {
                continue;
            }
            if let Some(s) = lookup(db, &n)? {
                out.push(s)
            }
        }
        Ok(())
    }
    fn node(&self, name: &str, v: i64) -> Result<Value> {
        let (db, id) = self.version(v)?;
        self.attach(&db)?;
        let mut node = lookup(&db, name)?.ok_or("Details were not indexed for this name")?;
        let base: Option<(String, Option<String>, Option<String>)> = db
            .query_row(
                "SELECT description,source_url,source_updated_at FROM nodes WHERE name=?",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()
            .map_err(err)?;
        let (desc, url, updated) = if node["custom"] != true && base.is_some() {
            let b = base.ok_or("Source record disappeared")?;
            b
        } else {
            (
                db.query_row(
                    "SELECT description FROM overlay.custom WHERE name=?",
                    [name],
                    |r| r.get(0),
                )
                .map_err(err)?,
                None,
                None,
            )
        };
        let edit: Option<String> = db
            .query_row(
                "SELECT description FROM overlay.edits WHERE name=?",
                [name],
                |r| r.get(0),
            )
            .optional()
            .map_err(err)?;
        node["description"] = json!(edit.as_ref().unwrap_or(&desc));
        node["edited"] = json!(edit.is_some());
        node["sourceUrl"] = json!(url);
        node["updated"] = json!(updated);
        node["version"] = json!(id);
        node["indexedAt"] = db
            .query_row("SELECT value FROM meta WHERE key='built_at'", [], |r| {
                r.get::<_, String>(0)
            })
            .map(Value::String)
            .unwrap_or(Value::Null);
        for (field, kind, reverse) in [
            ("parents", "child", true),
            ("children", "child", false),
            ("related", "related", false),
            ("crossReferences", "xref", false),
        ] {
            let sql = if reverse {
                "SELECT n.name,n.display_name,n.kind,n.coverage,n.child_count FROM edges e JOIN nodes n ON n.id=e.parent_id JOIN nodes p ON p.id=e.child_id WHERE p.name=? AND e.edge_type=? ORDER BY e.position"
            } else {
                "SELECT n.name,n.display_name,n.kind,n.coverage,n.child_count FROM edges e JOIN nodes n ON n.id=e.child_id JOIN nodes p ON p.id=e.parent_id WHERE p.name=? AND e.edge_type=? ORDER BY e.position"
            };
            let mut stmt = db.prepare(sql).map_err(err)?;
            let mut out = stmt
                .query_map([name, kind], summary)
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            self.enrich_summaries(&db, &mut out)?;
            self.add_custom_links(&db, name, kind, reverse, &mut out)?;
            node[field] = json!(out);
        }
        Ok(node)
    }
    fn search(&self, q: &str, kind: &str, v: i64) -> Result<Value> {
        if q.trim().chars().count() < 2 {
            return Ok(json!([]));
        }
        let (db, _) = self.version(v)?;
        self.attach(&db)?;
        let query = q
            .split_whitespace()
            .take(8)
            .map(|s| s.replace('"', ""))
            .filter(|s| !s.is_empty())
            .map(|s| format!("\"{s}\"*"))
            .collect::<Vec<_>>()
            .join(" AND ");
        if query.is_empty() {
            return Ok(json!([]));
        }
        let mut stmt=db.prepare("SELECT n.name,n.display_name,n.kind,n.coverage,n.child_count FROM nodes_fts JOIN nodes n ON n.id=nodes_fts.rowid WHERE nodes_fts MATCH ?1 AND (?2='' OR n.kind=?2 OR (?2='group' AND n.kind='section')) ORDER BY rank LIMIT 80").map_err(err)?;
        let mut out = stmt
            .query_map([query.as_str(), kind], summary)
            .map_err(err)?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(err)?;
        let mut stmt=db.prepare("SELECT c.name,c.display,c.kind,'full',0 FROM overlay.custom c LEFT JOIN overlay.edits e ON e.name=c.name WHERE (instr(lower(c.name),lower(?1)) OR instr(lower(c.display),lower(?1)) OR instr(lower(coalesce(e.description,c.description)),lower(?1))) AND (?2='' OR c.kind=?2) UNION SELECT n.name,n.display_name,n.kind,n.coverage,n.child_count FROM nodes n JOIN overlay.edits e ON n.name=e.name WHERE instr(lower(e.description),lower(?1)) AND (?2='' OR n.kind=?2 OR (?2='group' AND n.kind='section')) LIMIT 80").map_err(err)?;
        for row in stmt.query_map([q, kind], summary).map_err(err)? {
            let row = row.map_err(err)?;
            if !out.iter().any(|n| n["name"] == row["name"]) {
                out.push(row)
            }
        }
        let mut links = db.prepare("SELECT DISTINCT parent FROM overlay.custom_edges WHERE instr(lower(child),lower(?)) LIMIT 80").map_err(err)?;
        for name in links
            .query_map([q.replace(' ', "_")], |r| r.get::<_, String>(0))
            .map_err(err)?
        {
            if let Some(row) = lookup(&db, &name.map_err(err)?)? {
                if (kind.is_empty()
                    || row["kind"] == kind
                    || (kind == "group" && row["kind"] == "section"))
                    && !out.iter().any(|n| n["name"] == row["name"])
                {
                    out.push(row);
                }
            }
        }
        // Re-read summaries to report overlay child counts consistently.
        for row in &mut out {
            if let Some(node) = lookup(&db, text(row, "name"))? {
                *row = node;
            }
        }
        Ok(json!(out))
    }
    fn create(&self, a: &Value) -> Result<Value> {
        let name = crate::indexer::normalize(text(a, "name"));
        if name.is_empty() || name.starts_with("section:") {
            return Err("Enter a valid custom name".into());
        }
        let (db, _) = self.version(a["version"].as_i64().unwrap_or(0))?;
        self.attach(&db)?;
        let (current, _) = self.version(0)?;
        self.attach(&current)?;
        if lookup(&db, &name)?.is_some() || lookup(&current, &name)?.is_some() {
            return Err("That name already exists".into());
        }
        let kind = text(a, "kind");
        if !["tag", "group"].contains(&kind) {
            return Err("Invalid entry kind".into());
        }
        let mut r = self.registry()?;
        let tx = r.transaction().map_err(err)?;
        tx.execute(
            "INSERT INTO custom VALUES(?1,?2,?3,?4)",
            params![name, name.replace('_', " "), kind, text(a, "description")],
        )
        .map_err(err)?;
        let parent = if text(a, "parent").is_empty() {
            "tag_groups"
        } else {
            text(a, "parent")
        };
        if lookup(&db, parent)?.is_none() {
            return Err("Parent is unavailable in the selected index".into());
        }
        tx.execute(
            "INSERT INTO custom_edges VALUES(?1,?2,'child')",
            params![parent, name],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!(name))
    }
    fn edge(&self, a: &Value) -> Result<Value> {
        let p = text(a, "parent");
        let c = if text(a, "child").starts_with("section:") {
            text(a, "child").trim().to_string()
        } else {
            crate::indexer::normalize(text(a, "child"))
        };
        let k = text(a, "kind");
        if !["child", "related", "xref"].contains(&k) {
            return Err("Invalid relationship type".into());
        }
        let (db, _) = self.version(a["version"].as_i64().unwrap_or(0))?;
        self.attach(&db)?;
        if lookup(&db, p)?.is_none() || lookup(&db, &c)?.is_none() {
            return Err("Both endpoints must exist locally".into());
        }
        self.registry()?
            .execute(
                if a["remove"].as_bool().unwrap_or(false) {
                    "DELETE FROM custom_edges WHERE parent=?1 AND child=?2 AND kind=?3"
                } else {
                    "INSERT OR IGNORE INTO custom_edges VALUES(?1,?2,?3)"
                },
                params![p, c, k],
            )
            .map_err(err)?;
        Ok(json!(true))
    }
    fn delete_custom(&self, name: &str) -> Result<Value> {
        let mut r = self.registry()?;
        let tx = r.transaction().map_err(err)?;
        if tx
            .execute("DELETE FROM custom WHERE name=?", [name])
            .map_err(err)?
            == 0
        {
            return Err("Only custom entries can be deleted".into());
        }
        tx.execute(
            "DELETE FROM custom_edges WHERE parent=?1 OR child=?1",
            [name],
        )
        .map_err(err)?;
        tx.execute("DELETE FROM edits WHERE name=?", [name])
            .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(json!(true))
    }
    pub fn activate(
        &self,
        path: &Path,
        label: &str,
        current: bool,
        protected: bool,
    ) -> Result<i64> {
        validate(path)?;
        let mut r = self.registry()?;
        let tx = r.transaction().map_err(err)?;
        let id: i64 = tx
            .query_row(
                "SELECT coalesce(max(version),0)+1 FROM snapshots",
                [],
                |r| r.get(0),
            )
            .map_err(err)?;
        let file = format!("v{id}.sqlite");
        fs::copy(path, self.dir.join("versions").join(&file)).map_err(err)?;
        if current {
            tx.execute("UPDATE snapshots SET current=0", [])
                .map_err(err)?;
        }
        tx.execute(
            "INSERT INTO snapshots VALUES(?1,?2,?3,?4,?5,?6)",
            params![id, label, now(), protected, current, file],
        )
        .map_err(err)?;
        tx.commit().map_err(err)?;
        Ok(id)
    }
    pub fn retention(&self) -> Result<()> {
        let mut r = self.registry()?;
        let tx = r.transaction().map_err(err)?;
        let stale: Vec<(i64, String)> = {
            let mut s=tx.prepare("SELECT version,file FROM snapshots WHERE current=0 AND protected=0 ORDER BY version DESC LIMIT -1 OFFSET 2").map_err(err)?;
            let x = s
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(err)?
                .collect::<std::result::Result<_, _>>()
                .map_err(err)?;
            x
        };
        for (id, _) in &stale {
            tx.execute("DELETE FROM snapshots WHERE version=?", [id])
                .map_err(err)?;
        }
        tx.commit().map_err(err)?;
        for (_, f) in stale {
            let _ = fs::remove_file(self.dir.join("versions").join(f));
        }
        Ok(())
    }
    fn snapshot(&self, a: &Value) -> Result<Value> {
        let id = a["version"].as_i64().ok_or("Version required")?;
        let r = self.registry()?;
        let action = text(a, "action");
        match action {
            "protect" | "unprotect" => {
                r.execute(
                    "UPDATE snapshots SET protected=?1 WHERE version=?2",
                    params![action == "protect", id],
                )
                .map_err(err)?;
            }
            "rename" => {
                let label = text(a, "label").trim();
                if label.is_empty() {
                    return Err("Name is required".into());
                }
                r.execute(
                    "UPDATE snapshots SET label=?1 WHERE version=?2",
                    params![label, id],
                )
                .map_err(err)?;
            }
            "delete" => {
                if a["confirm"] != true {
                    return Err("Deletion requires confirmation".into());
                }
                let (file, current): (String, bool) = r
                    .query_row(
                        "SELECT file,current FROM snapshots WHERE version=?",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )
                    .map_err(err)?;
                if current {
                    return Err("Cannot delete current snapshot".into());
                }
                r.execute("DELETE FROM snapshots WHERE version=?", [id])
                    .map_err(err)?;
                let _ = fs::remove_file(self.dir.join("versions").join(file));
            }
            "restore" => {
                let file: String = r
                    .query_row("SELECT file FROM snapshots WHERE version=?", [id], |r| {
                        r.get(0)
                    })
                    .map_err(err)?;
                self.activate(
                    &self.dir.join("versions").join(file),
                    &format!("Restored from v{id}"),
                    true,
                    false,
                )?;
                self.retention()?;
            }
            _ => return Err("Unknown snapshot action".into()),
        }
        self.meta()
    }
    fn compare(&self, left: i64, right: i64) -> Result<Value> {
        let (a, left_id) = self.version(left)?;
        let (right_db, right_id) = self.version(right)?;
        drop(right_db);
        if left_id == right_id {
            return Ok(json!([
                "Added tags",
                "Removed tags",
                "Modified descriptions",
                "Added categories",
                "Removed categories",
                "Added relationships",
                "Removed relationships"
            ]
            .map(|label| json!({"label":label,"total":0,"sample":[]}))));
        }
        let r = self.registry()?;
        let file: String = r
            .query_row(
                "SELECT file FROM snapshots WHERE version=?",
                [right_id],
                |r| r.get(0),
            )
            .map_err(err)?;
        a.execute(
            "ATTACH DATABASE ? AS other",
            [self
                .dir
                .join("versions")
                .join(file)
                .to_string_lossy()
                .as_ref()],
        )
        .map_err(err)?;
        let queries=[("Added tags","SELECT name FROM other.nodes WHERE kind='tag' EXCEPT SELECT name FROM nodes WHERE kind='tag'"),("Removed tags","SELECT name FROM nodes WHERE kind='tag' EXCEPT SELECT name FROM other.nodes WHERE kind='tag'"),("Modified descriptions","SELECT a.name FROM nodes a JOIN other.nodes b ON a.name=b.name WHERE a.description<>b.description"),("Added categories","SELECT name FROM other.nodes WHERE kind<>'tag' EXCEPT SELECT name FROM nodes WHERE kind<>'tag'"),("Removed categories","SELECT name FROM nodes WHERE kind<>'tag' EXCEPT SELECT name FROM other.nodes WHERE kind<>'tag'")];
        let mut out = Vec::new();
        for (label, sql) in queries {
            out.push(diff_query(&a, label, sql)?)
        }
        for (label, first, second) in [
            ("Added relationships", "other.", ""),
            ("Removed relationships", "", "other."),
        ] {
            let select = |prefix: &str| {
                format!("SELECT p.name || ' → ' || c.name || ' [' || e.edge_type || ', position ' || e.position || ']' FROM {prefix}edges e JOIN {prefix}nodes p ON p.id=e.parent_id JOIN {prefix}nodes c ON c.id=e.child_id")
            };
            out.push(diff_query(
                &a,
                label,
                &format!("{} EXCEPT {}", select(first), select(second)),
            )?)
        }
        Ok(json!(out))
    }
    fn export(&self, a: &Value) -> Result<Value> {
        let dest = PathBuf::from(text(a, "path"));
        let tmp = dest.with_extension("zip.partial");
        let mode = text(a, "mode");
        let mut writer = zip::ZipWriter::new(fs::File::create(&tmp).map_err(err)?);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(9));
        let result = (|| -> Result<()> {
            let meta = self.meta()?;
            let snapshots: Vec<Value> = meta["snapshots"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| {
                    mode == "history"
                        || (mode != "overlay"
                            && if a["version"].as_i64().unwrap_or(0) == 0 {
                                s["current"] == true
                            } else {
                                s["version"] == a["version"]
                            })
                })
                .cloned()
                .collect();
            let manifest = json!({"format":"tag-codex-2","snapshots":snapshots,"overlay":self.overlay_json()?});
            writer.start_file("manifest.json", opts).map_err(err)?;
            writer
                .write_all(&serde_json::to_vec_pretty(&manifest).map_err(err)?)
                .map_err(err)?;
            for s in snapshots {
                let id = s["version"].as_i64().unwrap();
                writer
                    .start_file(format!("v{id}.sqlite"), opts)
                    .map_err(err)?;
                let mut f =
                    fs::File::open(self.dir.join(format!("versions/v{id}.sqlite"))).map_err(err)?;
                std::io::copy(&mut f, &mut writer).map_err(err)?;
            }
            writer.finish().map_err(err)?.sync_all().map_err(err)?;
            fs::rename(&tmp, &dest).map_err(err)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&tmp);
        }
        result?;
        Ok(json!(dest))
    }
    fn overlay_json(&self) -> Result<Value> {
        let r = self.registry()?;
        let mut out = json!({});
        for (table, columns) in [("custom", 4), ("edits", 2), ("custom_edges", 3)] {
            let mut stmt = r.prepare(&format!("SELECT * FROM {table}")).map_err(err)?;
            let rows = stmt
                .query_map([], |r| {
                    let mut row = Vec::new();
                    for i in 0..columns {
                        row.push(r.get::<_, String>(i)?)
                    }
                    Ok(row)
                })
                .map_err(err)?
                .collect::<std::result::Result<Vec<_>, _>>()
                .map_err(err)?;
            out[table] = json!(rows);
        }
        Ok(out)
    }
    fn import(&self, a: &Value) -> Result<Value> {
        let path = PathBuf::from(text(a, "path"));
        let file = fs::File::open(path).map_err(err)?;
        let mut archive = zip::ZipArchive::new(file).map_err(err)?;
        let mut manifest = String::new();
        archive
            .by_name("manifest.json")
            .map_err(err)?
            .take(32 * 1024 * 1024)
            .read_to_string(&mut manifest)
            .map_err(err)?;
        let m: Value = serde_json::from_str(&manifest).map_err(err)?;
        if m["format"] != "tag-codex-2" {
            return Err("This is not a compatible Danbooru Tag Indexer archive".into());
        }
        let snaps = m["snapshots"].as_array().ok_or("Invalid manifest")?;
        if snaps.len() > 1000 {
            return Err("Archive has too many snapshots".into());
        }
        let work = self.dir.join("import-staging");
        let _ = fs::remove_dir_all(&work);
        fs::create_dir(&work).map_err(err)?;
        let result = (|| -> Result<Value> {
            for (i, s) in snaps.iter().enumerate() {
                let old = s["version"].as_i64().ok_or("Invalid snapshot version")?;
                let mut entry = archive.by_name(&format!("v{old}.sqlite")).map_err(err)?;
                if entry.size() > 4 * 1024 * 1024 * 1024 {
                    return Err("Snapshot exceeds 4 GiB limit".into());
                }
                let p = work.join(format!("{i}.sqlite"));
                let mut f = fs::File::create(&p).map_err(err)?;
                std::io::copy(&mut entry, &mut f).map_err(err)?;
                validate(&p)?;
            }
            let mut r = self.registry()?;
            let tx = r.transaction().map_err(err)?;
            for (table, columns) in [("custom", 4), ("edits", 2), ("custom_edges", 3)] {
                let rows = m["overlay"][table].as_array().ok_or("Invalid overlay")?;
                for row in rows {
                    let vals = row.as_array().ok_or("Invalid overlay row")?;
                    if vals.len() != columns || vals.iter().any(|v| !v.is_string()) {
                        return Err("Invalid overlay values".into());
                    }
                    let values = vals.iter().map(|v| v.as_str().unwrap()).collect::<Vec<_>>();
                    if table == "custom" && !["tag", "group"].contains(&values[2]) {
                        return Err("Invalid custom type".into());
                    }
                    if table == "custom_edges" && !["child", "related", "xref"].contains(&values[2])
                    {
                        return Err("Invalid relationship type".into());
                    }
                    if table != "custom_edges" && a["overwrite"] != true {
                        let exists: i64 = tx
                            .query_row(
                                &format!("SELECT count(*) FROM {table} WHERE name=?"),
                                [values[0]],
                                |r| r.get(0),
                            )
                            .map_err(err)?;
                        if exists > 0 {
                            return Err("Archive contains conflicting custom edits. Enable overwrite only if you want to replace them.".into());
                        }
                    }
                    let marks = (0..columns).map(|_| "?").collect::<Vec<_>>().join(",");
                    tx.execute(
                        &format!("INSERT OR REPLACE INTO {table} VALUES({marks})"),
                        rusqlite::params_from_iter(values),
                    )
                    .map_err(err)?;
                }
            }
            let mut next: i64 = tx
                .query_row(
                    "SELECT coalesce(max(version),0)+1 FROM snapshots",
                    [],
                    |r| r.get(0),
                )
                .map_err(err)?;
            let activate = a["activate"] == true;
            let current_index = snaps.iter().position(|s| s["current"] == true).unwrap_or(0);
            for (i, s) in snaps.iter().enumerate() {
                let file = format!("v{next}.sqlite");
                fs::copy(
                    work.join(format!("{i}.sqlite")),
                    self.dir.join("versions").join(&file),
                )
                .map_err(err)?;
                let current = activate && i == current_index;
                if current {
                    tx.execute("UPDATE snapshots SET current=0", [])
                        .map_err(err)?;
                }
                tx.execute(
                    "INSERT INTO snapshots VALUES(?1,?2,?3,?4,?5,?6)",
                    params![
                        next,
                        text(s, "label"),
                        text(s, "created"),
                        s["protected"] == true,
                        current,
                        file
                    ],
                )
                .map_err(err)?;
                next += 1;
            }
            tx.commit().map_err(err)?;
            Ok(json!({"imported":snaps.len()}))
        })();
        let _ = fs::remove_dir_all(work);
        result
    }
}
fn summary(r: &rusqlite::Row) -> rusqlite::Result<Value> {
    Ok(
        json!({"name":r.get::<_,String>(0)?,"display":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"coverage":r.get::<_,String>(3)?,"children":r.get::<_,i64>(4)?}),
    )
}
fn lookup(db: &Connection, name: &str) -> Result<Option<Value>> {
    let base = db
        .query_row(
            "SELECT name,display_name,kind,coverage,child_count FROM nodes WHERE name=?",
            [name],
            summary,
        )
        .optional()
        .map_err(err)?;
    let mut custom = db
        .query_row(
            "SELECT name,display,kind,'full',0 FROM overlay.custom WHERE name=?",
            [name],
            summary,
        )
        .optional()
        .map_err(err)?;
    if let Some(ref mut c) = custom {
        c["custom"] = json!(true);
        c["children"] = base
            .as_ref()
            .map(|b| b["children"].clone())
            .unwrap_or(json!(0));
    }
    let mut result = custom.or(base);
    if let Some(ref mut n) = result {
        let extra: i64 = db
            .query_row(
                "SELECT count(*) FROM overlay.custom_edges WHERE parent=? AND kind='child'",
                [name],
                |r| r.get(0),
            )
            .map_err(err)?;
        n["children"] = json!(n["children"].as_i64().unwrap_or(0) + extra);
    }
    Ok(result)
}
fn diff_query(db: &Connection, label: &str, sql: &str) -> Result<Value> {
    let mut s = db.prepare(sql).map_err(err)?;
    let mut total = 0;
    let mut sample = Vec::new();
    for r in s.query_map([], |r| r.get::<_, String>(0)).map_err(err)? {
        let r = r.map_err(err)?;
        total += 1;
        if sample.len() < 150 {
            sample.push(r)
        }
    }
    Ok(json!({"label":label,"total":total,"sample":sample}))
}
pub fn counts(db: &Connection) -> Result<Vec<i64>> {
    db.query_row("SELECT (SELECT count(*) FROM nodes WHERE kind='tag'),(SELECT count(*) FROM nodes WHERE kind<>'tag'),(SELECT count(*) FROM edges)",[],|r|Ok(vec![r.get(0)?,r.get(1)?,r.get(2)?])).map_err(err)
}
pub fn validate(path: &Path) -> Result<()> {
    let db = ro(path)?;
    let integrity: String = db
        .query_row("PRAGMA integrity_check", [], |r| r.get(0))
        .map_err(err)?;
    if integrity != "ok" {
        return Err("Database integrity check failed".into());
    }
    let schema: String = db
        .query_row("SELECT value FROM meta WHERE key='schema'", [], |r| {
            r.get(0)
        })
        .map_err(err)?;
    if schema != "1" {
        return Err("Unsupported database schema".into());
    }
    let counts = counts(&db)?;
    if counts[0] < 1000 {
        return Err("Incomplete index: fewer than 1,000 tags".into());
    }
    let orphan:i64=db.query_row("SELECT count(*) FROM edges e LEFT JOIN nodes p ON p.id=e.parent_id LEFT JOIN nodes c ON c.id=e.child_id WHERE p.id IS NULL OR c.id IS NULL",[],|r|r.get(0)).map_err(err)?;
    if orphan != 0 {
        return Err("Broken relationship endpoints".into());
    }
    let required = ["tag_groups", "pool_groups", "list_of_meta-wikis"];
    for n in required {
        let exists: i64 = db
            .query_row("SELECT count(*) FROM nodes WHERE name=?", [n], |r| r.get(0))
            .map_err(err)?;
        if exists == 0 {
            return Err(format!("Required group missing: {n}"));
        }
    }
    let mut statement=db.prepare("WITH RECURSIVE tree(id,depth) AS(SELECT id,0 FROM nodes WHERE name='tag_groups' UNION SELECT e.child_id,t.depth+1 FROM tree t JOIN edges e ON e.parent_id=t.id WHERE e.edge_type='child' AND t.depth<2) SELECT lower(n.name || ' ' || n.display_name) FROM tree t JOIN nodes n ON n.id=t.id").map_err(err)?;
    let labels = statement
        .query_map([], |r| r.get::<_, String>(0))
        .map_err(err)?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(err)?
        .join(" ");
    for needle in [
        "visual",
        "copyright",
        "metatag",
        "pool_groups",
        "list_of_meta-wikis",
    ] {
        if !labels.contains(needle) {
            return Err(format!("Required root branch missing: {needle}"));
        }
    }
    let _: i64 = db
        .query_row(
            "SELECT count(*) FROM nodes_fts WHERE nodes_fts MATCH 'body'",
            [],
            |r| r.get(0),
        )
        .map_err(err)?;
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    fn fixture() -> (tempfile::TempDir, Arc<App>) {
        let tmp = tempfile::tempdir().unwrap();
        let seed = tmp.path().join("seed.sqlite");
        let db = Connection::open(&seed).unwrap();
        db.execute_batch("CREATE TABLE nodes(id INTEGER PRIMARY KEY,name TEXT UNIQUE,display_name TEXT,kind TEXT,description TEXT,source_url TEXT,source_updated_at TEXT,coverage TEXT,child_count INTEGER);CREATE TABLE edges(parent_id INTEGER,child_id INTEGER,edge_type TEXT,position INTEGER);CREATE TABLE meta(key TEXT,value TEXT);INSERT INTO meta VALUES('schema','1');CREATE VIRTUAL TABLE nodes_fts USING fts5(name,display_name,description);").unwrap();
        db.execute_batch("BEGIN").unwrap();
        for i in 1..=1100 {
            let name = match i {
                1 => "tag_groups".into(),
                2 => "pool_groups".into(),
                3 => "list_of_meta-wikis".into(),
                1098 => "visual characteristics".into(),
                1099 => "copyrights artists projects and media".into(),
                1100 => "metatags".into(),
                _ => format!("tag_{i}"),
            };
            db.execute(
                "INSERT INTO nodes VALUES(?1,?2,?2,?3,'Original description',NULL,NULL,'full',0)",
                params![i, name, if i <= 3 || i >= 1098 { "group" } else { "tag" }],
            )
            .unwrap();
            db.execute("INSERT INTO nodes_fts(rowid,name,display_name,description) VALUES(?1,?2,?2,'body')",params![i,name]).unwrap();
            if i > 1 {
                db.execute("INSERT INTO edges VALUES(1,?1,'child',?1)", [i])
                    .unwrap();
            }
        }
        db.execute_batch("COMMIT").unwrap();
        drop(db);
        let app = Arc::new(App::new(tmp.path().join("app"), seed).unwrap());
        (tmp, app)
    }
    fn call(app: &Arc<App>, op: &str, args: Value) -> Value {
        app.dispatch(op, &args).unwrap()
    }
    #[test]
    fn offline_navigation_edits_relationships() {
        let (_tmp, a) = fixture();
        assert_eq!(call(&a, "job", json!({}))["status"], "idle");
        assert!(!a.running.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(
            call(&a, "children", json!({})).as_array().unwrap().len(),
            1099
        );
        let n = call(
            &a,
            "create",
            json!({"name":"My tag","kind":"tag","description":"custom description","parent":"tag_4"}),
        );
        assert_eq!(n, "my_tag");
        let node = call(&a, "node", json!({"name":"my_tag"}));
        assert_eq!(node["parents"][0]["name"], "tag_4");
        call(
            &a,
            "edge",
            json!({"parent":"my_tag","child":"tag_5","kind":"related"}),
        );
        assert_eq!(
            call(&a, "node", json!({"name":"my_tag"}))["related"][0]["name"],
            "tag_5"
        );
        call(
            &a,
            "edit",
            json!({"name":"tag_4","description":"uniqueoverlayword"}),
        );
        assert!(call(&a, "search", json!({"query":"uniqueoverlayword"}))
            .as_array()
            .unwrap()
            .iter()
            .any(|x| x["name"] == "tag_4"));
        call(&a, "revert", json!({"name":"tag_4"}));
        assert_eq!(
            call(&a, "node", json!({"name":"tag_4"}))["description"],
            "Original description"
        );
        assert_eq!(call(&a, "job", json!({}))["status"], "idle");
    }
    #[test]
    fn protection_restore_retention_and_comparison() {
        let (_tmp, a) = fixture();
        call(&a, "snapshot", json!({"version":1,"action":"protect"}));
        for _ in 0..5 {
            call(&a, "snapshot", json!({"version":1,"action":"restore"}));
        }
        let m = call(&a, "meta", json!({}));
        assert_eq!(m["snapshots"].as_array().unwrap().len(), 4);
        assert!(m["snapshots"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["version"] == 1 && s["protected"] == true));
        let diff = call(&a, "compare", json!({"left":1,"right":6}));
        assert!(diff.as_array().unwrap().iter().all(|x| x["total"] == 0));
        let same = call(&a, "compare", json!({"left":6,"right":0}));
        assert_eq!(same.as_array().unwrap().len(), 7);
        assert!(same.as_array().unwrap().iter().all(|x| x["total"] == 0));
        assert!(a
            .dispatch(
                "snapshot",
                &json!({"version":6,"action":"delete","confirm":true})
            )
            .is_err());
        assert!(a
            .dispatch("snapshot", &json!({"version":1,"action":"delete"}))
            .is_err());
    }
    #[test]
    fn backup_roundtrip_and_conflict_atomicity() {
        let (tmp, a) = fixture();
        call(
            &a,
            "create",
            json!({"name":"custom","kind":"group","parent":"tag_groups"}),
        );
        call(&a, "edit", json!({"name":"tag_4","description":"Edited"}));
        call(&a, "snapshot", json!({"version":1,"action":"restore"}));
        let zip = tmp.path().join("backup.zip");
        call(&a, "export", json!({"path":zip,"mode":"history"}));
        let count = call(&a, "meta", json!({}))["snapshots"]
            .as_array()
            .unwrap()
            .len();
        assert!(a.dispatch("import", &json!({"path":zip})).is_err());
        assert_eq!(
            call(&a, "meta", json!({}))["snapshots"]
                .as_array()
                .unwrap()
                .len(),
            count
        );
        let r = call(&a, "import", json!({"path":zip,"overwrite":true}));
        assert_eq!(r["imported"], 2);
        assert_eq!(
            call(&a, "node", json!({"name":"tag_4"}))["description"],
            "Edited"
        );
        let edits = tmp.path().join("edits.zip");
        call(&a, "export", json!({"path":edits,"mode":"overlay"}));
        call(&a, "deleteCustom", json!({"name":"custom"}));
        call(&a, "import", json!({"path":edits,"overwrite":true}));
        assert!(call(&a, "node", json!({"name":"custom"}))["custom"] == true);
    }
    #[test]
    fn recovery_requires_manual_action() {
        let (tmp, a) = fixture();
        fs::write(
            a.dir.join("update.json"),
            r#"{"status":"fetch","pages":42,"after":123,"elapsed":2,"failures":[]}"#,
        )
        .unwrap();
        let data_dir = a.dir.clone();
        drop(a);
        let restarted = App::new(data_dir, tmp.path().join("seed.sqlite")).unwrap();
        assert_eq!(restarted.job.lock().unwrap()["status"], "interrupted");
        assert!(!restarted.running.load(std::sync::atomic::Ordering::SeqCst));
    }
    #[test]
    fn custom_relationships_survive_reassigned_source_ids() {
        let (tmp, a) = fixture();
        call(
            &a,
            "create",
            json!({"name":"my_child","kind":"tag","parent":"tag_4"}),
        );
        call(
            &a,
            "edge",
            json!({"parent":"my_child","child":"tag_5","kind":"xref"}),
        );
        let next = tmp.path().join("reordered.sqlite");
        fs::copy(a.dir.join("versions/v1.sqlite"), &next).unwrap();
        let db = Connection::open(&next).unwrap();
        db.execute_batch("UPDATE nodes SET id=-id;UPDATE nodes SET id=-id+5000;UPDATE edges SET parent_id=parent_id+5000,child_id=child_id+5000;UPDATE nodes_fts SET rowid=rowid+5000;").unwrap();
        drop(db);
        a.activate(&next, "Reordered index", true, false).unwrap();
        let node = call(&a, "node", json!({"name":"my_child"}));
        assert_eq!(node["parents"][0]["name"], "tag_4");
        assert_eq!(node["crossReferences"][0]["name"], "tag_5");
        assert!(call(&a, "children", json!({"name":"tag_4"}))
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "my_child"));
    }
    #[test]
    fn invalid_database_never_activates() {
        let (tmp, a) = fixture();
        let bad = tmp.path().join("bad.sqlite");
        fs::write(&bad, b"not a database").unwrap();
        assert!(a.activate(&bad, "bad", true, false).is_err());
        assert_eq!(a.meta().unwrap()["snapshots"][0]["version"], 1);
    }
    #[test]
    fn overlay_tree_counts_and_search_follow_custom_edits() {
        let (_tmp, a) = fixture();
        call(
            &a,
            "create",
            json!({"name":"extra_child","kind":"tag","parent":"tag_4","description":"before"}),
        );
        let children = call(&a, "children", json!({"name":"tag_groups"}));
        assert_eq!(
            children
                .as_array()
                .unwrap()
                .iter()
                .find(|n| n["name"] == "tag_4")
                .unwrap()["children"],
            1
        );
        call(
            &a,
            "edit",
            json!({"name":"extra_child","description":"uniquenewdescription"}),
        );
        assert!(call(&a, "search", json!({"query":"uniquenewdescription"}))
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "extra_child"));
        assert!(call(&a, "search", json!({"query":"extra child"}))
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "tag_4"));
    }
    #[test]
    fn comparison_detects_reordered_relationships() {
        let (tmp, a) = fixture();
        let next = tmp.path().join("reordered.sqlite");
        fs::copy(a.dir.join("versions/v1.sqlite"), &next).unwrap();
        let db = Connection::open(&next).unwrap();
        db.execute("UPDATE edges SET position=9000 WHERE child_id=4", [])
            .unwrap();
        drop(db);
        a.activate(&next, "Reordered", true, false).unwrap();
        let diff = call(&a, "compare", json!({"left":1,"right":2}));
        assert!(diff
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["label"] == "Added relationships" && n["total"] == 1));
        assert!(diff
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["label"] == "Removed relationships" && n["total"] == 1));
    }
    #[test]
    fn overwriting_export_produces_a_valid_archive() {
        let (tmp, a) = fixture();
        let dest = tmp.path().join("export.zip");
        call(&a, "export", json!({"path":dest,"mode":"overlay"}));
        call(&a, "edit", json!({"name":"tag_4","description":"latest"}));
        call(&a, "export", json!({"path":dest,"mode":"overlay"}));
        let mut zip = zip::ZipArchive::new(fs::File::open(dest).unwrap()).unwrap();
        let mut manifest = String::new();
        zip.by_name("manifest.json")
            .unwrap()
            .read_to_string(&mut manifest)
            .unwrap();
        assert!(manifest.contains("latest"));
    }
    #[test]
    fn future_source_name_collision_preserves_custom_description() {
        let (tmp, a) = fixture();
        call(
            &a,
            "create",
            json!({"name":"future_tag","kind":"tag","parent":"tag_groups","description":"My original custom text"}),
        );
        let next = tmp.path().join("future.sqlite");
        fs::copy(a.dir.join("versions/v1.sqlite"), &next).unwrap();
        let db = Connection::open(&next).unwrap();
        db.execute("INSERT INTO nodes VALUES(2000,'future_tag','future tag','tag','Upstream text',NULL,NULL,'full',0)",[]).unwrap();
        drop(db);
        a.activate(&next, "Future source", true, false).unwrap();
        let node = call(&a, "node", json!({"name":"future_tag"}));
        assert_eq!(node["description"], "My original custom text");
        assert_eq!(node["custom"], true);
    }
}
