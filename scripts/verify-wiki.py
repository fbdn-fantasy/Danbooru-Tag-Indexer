"""Capture public wiki source for release verification (never runs in the app)."""
import json
import pathlib
import urllib.parse
import urllib.request
import time

pages = ['tag_groups', 'tag_group:body_parts', 'tag_group:face_tags', 'tag_group:hair',
         'tag_group:feet', 'toes', 'toenails', 'tag_group:ass', 'ass',
         'tag_group:fighting_games', 'tag_group:metatags', 'pool_groups',
         'list_of_official_mascots', 'list_of_pokemon_trainer_classes']
target = pathlib.Path(__file__).resolve().parents[1] / 'tests/fixtures'
target.mkdir(parents=True, exist_ok=True)
result = []
for page in pages:
    url = 'https://danbooru.donmai.us/wiki_pages/' + urllib.parse.quote(page, safe='') + '.json'
    req = urllib.request.Request(url, headers={'User-Agent': 'DanbooruTagIndexer/5.0 verification'})
    with urllib.request.urlopen(req, timeout=40) as response:
        row = json.load(response)
    result.append({key: row.get(key) for key in ['id', 'title', 'body', 'updated_at']})
    print(page, row['updated_at'], flush=True)
    time.sleep(0.3)
(target / 'wiki-reference.json').write_text(json.dumps(result, indent=2), encoding='utf-8')
