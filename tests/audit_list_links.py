"""Audit listed wiki targets across the entire shipped catalog, not sample pages."""
import collections, json, pathlib, re, sqlite3, sys
path=pathlib.Path(sys.argv[1])
db=sqlite3.connect(f'file:{path.resolve()}?mode=ro',uri=True)
assert db.execute('PRAGMA integrity_check').fetchone()[0]=='ok'
names={r[0] for r in db.execute('SELECT name FROM nodes')}
link=re.compile(r'\[\[([^\[\]|]+)(?:\|[^\[\]]*)?\]\]')
start=re.compile(r'^\s*(?:[-#]+\s+|\[\[|\|)')
missing=collections.defaultdict(list)
pages=occurrences=0
for name,body in db.execute("SELECT name,description FROM nodes WHERE kind!='section'"):
    seen=False
    for line in body.splitlines():
        if not start.match(line):continue
        for target in link.findall(line):
            target='_'.join(target.split('#',1)[0].strip().lower().split())
            target='_'.join(part for part in target.split('_') if part)
            seen=True;occurrences+=1
            if target not in names:missing[target].append(name)
    pages+=seen
result={'integrity':'ok','nodes':len(names),'pages_with_listed_links':pages,
        'listed_link_occurrences':occurrences,'missing_target_names':len(missing),
        'missing_examples':dict(list(missing.items())[:30]),
        'coverage':dict(db.execute('SELECT coverage,count(*) FROM nodes GROUP BY coverage'))}
print(json.dumps(result,indent=2))
assert not missing, 'Listed wiki targets missing from the catalog'
