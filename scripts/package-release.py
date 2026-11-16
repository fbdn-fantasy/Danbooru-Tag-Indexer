"""Package an already-built release; keep platform folders to installer + source."""
import argparse, hashlib, json, pathlib, shutil, sqlite3, zipfile
parser=argparse.ArgumentParser()
parser.add_argument('--output',required=True)
parser.add_argument('--windows',required=True)
args=parser.parse_args()
root=pathlib.Path(__file__).resolve().parents[1]
out=pathlib.Path(args.output).resolve()
version=json.loads((root/'package.json').read_text())['version']
def digest(path):
    with path.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
packages={
    'Windows':(pathlib.Path(args.windows),f'Danbooru-Tag-Indexer-{version}-x64-setup.exe','windows'),
    'Linux/DEB':(next((root/'src-tauri/target/release/bundle/deb').glob('*.deb')),f'danbooru-tag-indexer_{version}_amd64.deb','deb'),
    'Linux/RPM':(next((root/'src-tauri/target/release/bundle/rpm').glob('*.rpm')),f'danbooru-tag-indexer-{version}-1.x86_64.rpm','rpm'),
}
excluded={'node_modules','dist','target','target-windows','gen','.git','__pycache__','.cache','test-results'}
source=[]
for file in root.rglob('*'):
    relative=file.relative_to(root)
    if any(p in excluded for p in relative.parts) or not file.is_file():continue
    if file.suffix in {'.log','.sqlite','.partial','.pdb','.exe','.deb','.rpm','.zip','.pyc'}:continue
    if file.name.endswith(('-journal','-wal','-shm')):continue
    source.append((file,relative))
archive=out/'source-staging.zip'
out.mkdir(parents=True,exist_ok=True)
with zipfile.ZipFile(archive,'w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
    for file,relative in sorted(source):z.write(file,'Danbooru-Tag-Indexer/'+relative.as_posix())
with zipfile.ZipFile(archive) as z:
    assert z.testzip() is None
    assert 'Danbooru-Tag-Indexer/src-tauri/resources/codex-v1.sqlite.gz' in z.namelist()
source_hash=digest(archive)
report={'version':version,'source_sha256':source_hash,'packages':{},'source_files':{str(r):digest(f) for f,r in source}}
for folder,(package,name,platform) in packages.items():
    destination=out/folder;destination.mkdir(parents=True,exist_ok=True)
    installer=destination/name;shutil.copy2(package,installer)
    zipped=destination/f'Danbooru-Tag-Indexer-{version}-{platform}-source.zip'
    shutil.copy2(archive,zipped)
    assert digest(zipped)==source_hash
    report['packages'][folder]={'installer':name,'sha256':digest(installer),'bytes':installer.stat().st_size,'source':zipped.name}
    assert {p.name for p in destination.iterdir()}=={installer.name,zipped.name}
archive.unlink()
db=sqlite3.connect(root/'src-tauri/resources/codex-v1.sqlite')
report['catalog_metadata']=dict(db.execute('SELECT key,value FROM meta'))
assert report['catalog_metadata']['scope']=='tag_groups-listed-links'
report['catalog_sha256']=digest(root/'src-tauri/resources/codex-v1.sqlite')
verification=out/'Verification';verification.mkdir(exist_ok=True)
with zipfile.ZipFile(verification/'Danbooru-Tag-Indexer-5.0.0-verification.zip','w',zipfile.ZIP_DEFLATED,compresslevel=9) as z:
    z.writestr('release-verification.json',json.dumps(report,indent=2))
    for name in ['test-frontend.log','test-scoped-rust.log','test-native-final.log','rebuild-scoped.log','build-linux.log','build-windows.log']:
        p=root/name
        if p.exists():z.write(p,name)
    for p in (root/'test-results').rglob('*'):
        if p.is_file() and (p.name in {'catalog-list-audit.json','catalog-scope.json','package-payloads.json'} or p.suffix=='.png' and 'native-final' in p.parts):
            z.write(p,'test-results/'+p.relative_to(root/'test-results').as_posix())
print(json.dumps(report['packages'],indent=2))
