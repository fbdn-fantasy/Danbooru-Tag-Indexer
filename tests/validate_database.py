"""Read-only validation of the exact SQLite resource shipped in the RPM."""
import collections,hashlib,json,pathlib,sqlite3,sys
path=pathlib.Path(sys.argv[1]);db=sqlite3.connect(f'file:{path.resolve()}?mode=ro',uri=True)
assert db.execute('pragma integrity_check').fetchone()[0]=='ok'
nodes={r[0]:r[1:] for r in db.execute('select id,name,display_name,kind,coverage from nodes')}
by_name={r[0]:i for i,r in nodes.items()};root=by_name['tag_groups']
adj=collections.defaultdict(list);edge_count=0
for parent,child,kind in db.execute('select parent_id,child_id,edge_type from edges'):
    assert parent in nodes and child in nodes
    if kind=='child':adj[parent].append(child)
    edge_count+=1
queue=collections.deque([(root,0)]);depths={root:0}
while queue:
    parent,depth=queue.popleft()
    for child in adj[parent]:
        if child not in depths:depths[child]=depth+1;queue.append((child,depth+1))
required={name:by_name[name] in depths for name in ['tag_groups','pool_groups','list_of_meta-wikis']};assert all(required.values())
root_sections=[nodes[i][1] for i in adj[root]]
assert any('visual' in s.lower() for s in root_sections)
assert any('copyright' in s.lower() for s in root_sections)
assert any('see also' in s.lower() for s in root_sections)
search=db.execute("select n.name from nodes_fts f join nodes n on n.id=f.rowid where nodes_fts match '\"egyptian\" AND \"toe\"' order by rank limit 50").fetchall()
assert ('egyptian_toe',) in search
result={'sha256':hashlib.file_digest(path.open('rb'),'sha256').hexdigest(),'integrity':'ok','nodes':len(nodes),'tags':sum(r[2]=='tag' for r in nodes.values()),'categories':sum(r[2]!='tag' for r in nodes.values()),'relationships':edge_count,'unindexed':sum(r[3]=='unindexed' for r in nodes.values()),'reachable_by_child_edges':len(depths),'maximum_shortest_path_depth':max(depths.values()),'root_sections':root_sections,'required_groups_reachable':required,'fts_egyptian_toe':True,'bytes':path.stat().st_size}
print(json.dumps(result,indent=2))
