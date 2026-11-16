"""Launch the native regression in isolated XDG storage under xvfb-run."""
import os, pathlib, subprocess, time, tempfile
root=pathlib.Path(__file__).resolve().parents[1]
output=root/'test-results/native-final'
output.mkdir(parents=True,exist_ok=True)
with tempfile.TemporaryDirectory(prefix='indexer-v5-native-') as tmp:
    stage=pathlib.Path(tmp)
    package=next((root/'src-tauri/target/release/bundle/deb').glob('*.deb'))
    subprocess.run(['dpkg-deb','--extract',str(package),str(stage)],check=True)
    env=os.environ.copy()
    env.update(XDG_DATA_HOME=str(stage/'data'),XDG_CONFIG_HOME=str(stage/'config'),
               XDG_CACHE_HOME=str(stage/'cache'),TAURI_AUTOMATION='true',TAURI_WEBVIEW_AUTOMATION='true',
               LIBGL_ALWAYS_SOFTWARE='1',GDK_BACKEND='x11',
               WEBKIT_DISABLE_COMPOSITING_MODE='1',TEST_OUTPUT=str(output),
               APP_BINARY=str(stage/'usr/bin/danbooru-tag-indexer'))
    with (output/'driver.log').open('w') as log:
        driver=subprocess.Popen(['WebKitWebDriver','--port=4445'],env=env,stdout=log,stderr=log)
        try:
            time.sleep(1)
            subprocess.run(['python3',str(root/'tests/native_navigation.py')],env=env,check=True)
        finally:
            driver.terminate()
            driver.wait(timeout=15)
