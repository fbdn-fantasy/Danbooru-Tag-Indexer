"""Exercise the real Tauri window through tauri-driver/WebKitWebDriver.
Run with a fresh XDG_DATA_HOME and tauri-driver listening on localhost:4444.
"""
import base64,json,os,pathlib,time,urllib.request
BASE=os.environ.get('WEBDRIVER_URL','http://127.0.0.1:4444')
def request(method,path,data=None):
    req=urllib.request.Request(BASE+path,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'},method=method)
    with urllib.request.urlopen(req,timeout=120) as response: result=json.load(response)
    if isinstance(result.get('value'),dict) and result['value'].get('error'): raise RuntimeError(result['value'])
    return result.get('value',result)
app=os.environ['APP_BINARY']
options = {'webkitgtk:browserOptions': {'binary': app}} if os.environ.get('WEBKIT_DIRECT') else {'tauri:options': {'application': app}}
session=request('POST','/session',{'capabilities':{'alwaysMatch':options}})
sid=session['sessionId'];prefix='/session/'+sid
request('POST',prefix+'/timeouts',{'script':120000,'pageLoad':120000,'implicit':0})
def js(script,args=[]):return request('POST',prefix+'/execute/sync',{'script':script,'args':args})
def wait(script,timeout=30):
    deadline=time.monotonic()+timeout
    while time.monotonic()<deadline:
        try:
            result=js(script)
            if result:return result
        except Exception:pass
        time.sleep(.25)
    raise AssertionError('Timed out: '+script+'\n'+str(js('return document.body.innerText')))
def click(label):
    assert js("const b=[...document.querySelectorAll('button')].find(e=>e.textContent.trim()===arguments[0]);if(b){b.click();return true}return false",[label]),label

def api(op,args={}):
    return request('POST',prefix+'/execute/async',{'script':"const cb=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('api',{op:arguments[0],args:arguments[1]}).then(x=>cb({ok:x})).catch(e=>cb({error:String(e)}))",'args':[op,args]})
root=pathlib.Path(__file__).resolve().parents[1]
output=pathlib.Path(os.environ.get('TEST_OUTPUT',str(root/'test-results')))
output.mkdir(parents=True,exist_ok=True)
try:
    wait("return document.querySelectorAll('.root-cards button').length>=3",60)
    assert api('job')['ok']['status']=='idle'
    assert js("return !document.body.innerText.includes('Offline ready') && document.querySelector('.support').textContent.trim()==='Support Me'")
    wait("return document.querySelector('.support img').naturalWidth > 0")
    js("document.querySelector('.search input').focus()")
    assert js("const input=document.querySelector('.search input'); const box=document.querySelector('.search'); const i=input.getBoundingClientRect(), b=box.getBoundingClientRect(); return getComputedStyle(input).outlineStyle==='none' && getComputedStyle(box).boxShadow.includes('inset') && i.left>=b.left && i.right<=b.right")
    assert not list(pathlib.Path(os.environ['XDG_DATA_HOME']).rglob('update.json'))
    js("document.querySelector('[aria-label=\"Expand Visual characteristics\"]').click()")
    wait("return document.querySelectorAll('.tree-row').length>5")
    (output/'native-dark.png').write_bytes(base64.b64decode(request('GET',prefix+'/screenshot')))
    js("const e=document.querySelector('[aria-label=\"Search encyclopedia\"]');e.value='egyptian toe';e.dispatchEvent(new Event('input',{bubbles:true}));")
    wait("return document.querySelector('.result-grid')?.innerText.toLowerCase().includes('egyptian toe')")
    js("[...document.querySelectorAll('.result-grid button')].find(e=>e.textContent.toLowerCase().includes('egyptian toe')).click()")
    wait("return document.querySelector('h1')?.textContent.toLowerCase().includes('egyptian toe')")
    click('Edit description')
    js("const e=document.querySelector('textarea.editor');e.value='Native UI regression edit';e.dispatchEvent(new Event('input',{bubbles:true}));")
    click('Save local edit')
    wait("return document.querySelector('.rich')?.textContent.includes('Native UI regression edit')")
    assert api('node',{'name':'egyptian_toe'})['ok']['description']=='Native UI regression edit'
    click('＋ Custom entry')
    wait("return document.querySelector('form.modal input')")
    js("const e=document.querySelector('form.modal input');e.value='ui_smoke_custom';e.dispatchEvent(new Event('input',{bubbles:true}));")
    click('Create entry')
    wait("return document.querySelector('h1')?.textContent.includes('ui smoke custom')")
    assert api('node',{'name':'ui_smoke_custom'})['ok']['parents'][0]['name']=='egyptian_toe'
    js("document.querySelector('[title=\"Toggle theme\"]').click()")
    assert js("return document.querySelector('.app').dataset.theme")=='light'
    time.sleep(.3)
    (output/'native-light.png').write_bytes(base64.b64decode(request('GET',prefix+'/screenshot')))
    click('Index history');wait("return document.querySelector('table tbody tr')")
    click('Keep permanently');wait("return document.querySelector('.badge')?.textContent.includes('Protected')")
    click('Compare versions');wait("return document.querySelectorAll('.diff-grid details').length===7",60)
    assert all(d['total']==0 for d in api('compare',{'left':1,'right':1})['ok'])
    (output/'native-history.png').write_bytes(base64.b64decode(request('GET',prefix+'/screenshot')))
    archive=str(pathlib.Path(os.environ['XDG_DATA_HOME'])/'ui-edits.zip')
    assert 'ok' in api('export',{'mode':'overlay','path':archive})
    assert pathlib.Path(archive).is_file()
    assert 'ok' in api('import',{'path':archive,'overwrite':True})
    assert api('job')['ok']['status']=='idle'
    assert not list(pathlib.Path(os.environ['XDG_DATA_HOME']).rglob('update.json'))
    resources=js("return performance.getEntriesByType('resource').map(x=>x.name)")
    assert not any('danbooru' in r for r in resources)
    print(json.dumps({'native_ui':'PASS','checks':['first launch','populated roots','search','description edit','custom entry and source parent','light theme','snapshot protection','version comparison','ZIP edits export/import','no update on startup/navigation'],'resources':resources},indent=2))
finally:
    request('DELETE',prefix)
