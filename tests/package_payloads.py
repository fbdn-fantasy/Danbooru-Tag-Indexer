"""Validate self-contained payloads of the three installable packages."""
import hashlib,json,pathlib,subprocess,tempfile,sys
root=pathlib.Path(__file__).resolve().parents[1]
windows=pathlib.Path(sys.argv[1])
def run(*args):return subprocess.check_output(args,stderr=subprocess.STDOUT,text=True)
def sha(p):
    with p.open('rb') as f:return hashlib.file_digest(f,'sha256').hexdigest()
expected=sha(root/'src-tauri/resources/codex-v1.sqlite')
report={}
with tempfile.TemporaryDirectory(prefix='indexer-payloads-') as tmp:
    stage=pathlib.Path(tmp)
    deb=next((root/'src-tauri/target/release/bundle/deb').glob('*.deb'))
    rpm=next((root/'src-tauri/target/release/bundle/rpm').glob('*.rpm'))
    run('dpkg-deb','-x',str(deb),str(stage/'deb'))
    (stage/'rpm').mkdir()
    with subprocess.Popen(['rpm2cpio',str(rpm)],stdout=subprocess.PIPE) as pipe:
        subprocess.run(['cpio','-id','--quiet'],cwd=stage/'rpm',stdin=pipe.stdout,check=True)
        pipe.stdout.close()
        assert pipe.wait()==0
    run('7z','x','-y','-o'+str(stage/'windows'),str(windows))
    for platform in ['deb','rpm','windows']:
        database=next((stage/platform).rglob('codex-v1.sqlite'))
        assert sha(database)==expected,platform
        binary=next((stage/platform).rglob('danbooru-tag-indexer'+('.exe' if platform=='windows' else '')))
        assert binary.stat().st_size>1000000
        frontend=next((root/'dist/assets').glob('index-*.js')).name
        assert frontend.encode() in binary.read_bytes(), (platform, 'stale embedded frontend')
        report[platform]={'catalog_sha256':expected,'binary_bytes':binary.stat().st_size,'frontend':frontend}
    assert next((stage/'windows').rglob('MicrosoftEdgeWebView2RuntimeInstaller.exe')).stat().st_size>100000000
    report['deb']['metadata']=run('dpkg-deb','-f',str(deb),'Package','Version','Architecture','Depends')
    report['rpm']['metadata']=run('rpm','-qp','--queryformat','%{NAME} %{VERSION} %{ARCH}',str(rpm))
    report['rpm']['dependencies']=run('rpm','-qp','--requires',str(rpm))
    report['windows']['imports']=run('llvm-readobj','--coff-imports',str(stage/'windows/danbooru-tag-indexer.exe'))
    assert 'VCRUNTIME140.dll' not in report['windows']['imports']
(root/'test-results/package-payloads.json').write_text(json.dumps(report,indent=2))
print('PASS: all three installers contain the exact scoped catalog and application; Windows includes WebView2 runtime.')
