"""End-to-end test of the packaged Tauri app via WebKitWebDriver.

Use isolated XDG directories; this test never opens the user's real saved data.
"""
import base64, json, os, pathlib, time, urllib.request
BASE=os.environ.get('WEBDRIVER_URL','http://127.0.0.1:4445')
def request(method,path,data=None):
    req=urllib.request.Request(BASE+path,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'},method=method)
    with urllib.request.urlopen(req,timeout=120) as r: result=json.load(r)
    value=result.get('value',result)
    if isinstance(value,dict) and value.get('error'): raise RuntimeError(value)
    return value
session=request('POST','/session',{'capabilities':{'alwaysMatch':{'webkitgtk:browserOptions':{'binary':os.environ['APP_BINARY']}}}})
prefix='/session/'+session['sessionId']
request('POST',prefix+'/timeouts',{'script':120000,'pageLoad':120000,'implicit':0})
def js(script,args=[]):return request('POST',prefix+'/execute/sync',{'script':script,'args':args})
def wait(script):
    deadline=time.monotonic()+45
    while time.monotonic()<deadline:
        if js(script):return
        time.sleep(.15)
    raise AssertionError(script+'\n'+str(js('return document.body.innerText')))
def click(selector):
    wait('return !!document.querySelector('+json.dumps(selector)+')')
    js('document.querySelector(arguments[0]).click()',[selector])
def api(op,args={}):
    result=request('POST',prefix+'/execute/async',{'script':"const cb=arguments[arguments.length-1];window.__TAURI_INTERNALS__.invoke('api',{op:arguments[0],args:arguments[1]}).then(x=>cb({ok:x})).catch(e=>cb({error:String(e)}))",'args':[op,args]})
    assert 'error' not in result,result
    return result['ok']
def search(query):
    js("const e=document.querySelector('[aria-label=\"Search encyclopedia\"]');e.value=arguments[0];e.dispatchEvent(new Event('input',{bubbles:true}));",[query])
    wait("return document.querySelector('.search-outlines .wiki-branch')")
def children(selector):return js("return [...document.querySelector(arguments[0]).querySelectorAll(':scope>.wiki-body>.wiki-tree>.wiki-branch')].map(e=>e.dataset.name)",[selector])
out=pathlib.Path(os.environ['TEST_OUTPUT']);out.mkdir(parents=True,exist_ok=True)
def screenshot(name): (out/name).write_bytes(base64.b64decode(request('GET',prefix+'/screenshot')))
try:
    wait("return document.querySelectorAll('.tree>.wiki-tree>.wiki-branch').length===5")
    assert api('job')['status']=='idle'
    for existing in api('favorites'):
        assert existing['name']=='egyptian_toe' and existing['description'] and existing['paths']
        api('favorite',{'name':existing['name'],'remove':True})
    click('.tree [aria-label="Expand Visual characteristics"]')
    click('.tree [aria-label="Expand Body"]')
    click('.tree [data-name="tag_group:body_parts"]>.wiki-row>.wiki-label')
    body='.tree [data-name="tag_group:body_parts"]'
    assert children(body)==['tag_group:ass','tag_group:breasts_tags','tag_group:face_tags','tag_group:hair','tag_group:hands','tag_group:feet','tag_group:neck_and_neckwear','on','tag_group:posture','tag_group:pussy','tag_group:shoulders','tag_group:skin_color','tag_group:wings']
    click(body+' [data-name="tag_group:face_tags"]>.wiki-row>.wiki-label')
    assert children(body+' [data-name="tag_group:face_tags"]')==['tag_group:ears_tags','tag_group:eyes_tags','eyebrows','nose']
    click(body+' [data-name="tag_group:hair"]>.wiki-row>.wiki-label')
    assert children(body+' [data-name="tag_group:hair"]')==['tag_group:hair_color','tag_group:hair_styles']
    screenshot('main-hierarchy.png')
    click(body+' [data-name="tag_group:feet"]>.wiki-row>.wiki-label')
    feet=body+' [data-name="tag_group:feet"]'
    wait('return !!document.querySelector('+json.dumps(feet+' [data-name="toes"]')+')')
    toes=feet+' [data-name="toes"]'
    click(toes+'>.wiki-row>.wiki-label')
    wait('return document.querySelector('+json.dumps(toes)+').innerText.includes("Egyptian toe")')
    wait("return document.querySelector('main article .entry-name')?.textContent==='toes'")
    assert js("return document.querySelector('main article').innerText.includes('Egyptian toe')")
    click(toes+'>.wiki-row>.wiki-label')
    assert js('return document.querySelector(arguments[0]).getAttribute("aria-expanded")=="true"',[toes+'>.wiki-row>.wiki-arrow'])
    text=js('return document.querySelector(arguments[0]).innerText',[toes])
    for value in ['Digits of the','Related tags','Appearance','Actions','See also','Egyptian toe','Long toes','Toe socks','Toe sucking','Prehensile toes','Prehensile feet']: assert value in text,value
    nails=toes+' [data-name="toenails"]'
    click(nails+'>.wiki-row>.wiki-arrow')
    wait('return document.querySelector('+json.dumps(nails)+').innerText.includes("Fingernails")')
    for name in ['long_toenails','sharp_toenails','toenail_polish']:
        assert js('return document.querySelectorAll(arguments[0]).length',[nails+f' [data-name="{name}"]'])==1
    assert js("return document.querySelector('main article .entry-name').textContent==='toes'")
    click(nails+'>.wiki-row>.wiki-label')
    wait("return document.querySelector('main article .entry-name')?.textContent==='toenails'")
    click(toes+' [data-name="egyptian_toe"]>.wiki-row>.wiki-label')
    wait("return document.querySelector('main article .entry-name')?.textContent==='egyptian_toe'")
    screenshot('tag-opens-main-panel.png')
    click(toes+' [data-name="egyptian_toe"]>.wiki-row>.wiki-star')
    wait("return document.querySelector('nav').innerText.includes('Favorites 1')")
    saved=api('favorites')[0]
    assert any(any(p['name']=='tag_group:feet' for p in path) for path in saved['paths'])
    assert saved['description']
    copy_selector=toes+' [data-name="egyptian_toe"]>.wiki-row>.wiki-copy'
    element=request('POST',prefix+'/element',{'using':'css selector','value':copy_selector})
    request('POST',prefix+'/element/'+element['element-6066-11e4-a52e-4f735466cecf']+'/click',{})
    wait("return document.querySelector('.notice')?.innerText.includes('Copied egyptian toe')")
    search('toes')
    wait("return document.querySelector('.search-outlines>.wiki-tree>.wiki-branch')?.dataset.name==='toes'")
    click('.search-outlines [data-name="toes"]>.wiki-row>.wiki-arrow')
    wait("return document.querySelector('.search-outlines [data-name=toes]').innerText.includes('Egyptian toe')")
    assert api('search',{'query':'toes','kind':'tag'})[0]['name']=='toes'
    screenshot('toes-search.png')
    # Repeated wiki links stay finite and do not fetch their page again.
    branch='.search-outlines [data-name="toes"] [data-name="greek_toe"]'
    click(branch+'>.wiki-row>.wiki-arrow')
    repeat=branch+' [data-name="toes"]'
    click(repeat+'>.wiki-row>.wiki-arrow')
    wait('return document.querySelector('+json.dumps(repeat)+').innerText.includes("already expanded above")')
    assert js('return !document.querySelector(arguments[0])',[repeat+'>.wiki-body>.wiki-tree'])
    click('.search-outlines [data-name="toes"]>.wiki-row>.wiki-label')
    wait("return document.querySelector('main article .entry-name')?.textContent==='toes' && !document.querySelector('.search-outlines')")
    click('main article [data-name="toenails"]>.wiki-row>.wiki-label')
    wait("return document.querySelector('main article .entry-name')?.textContent==='toenails'")
    js("document.querySelector('[aria-label=\"Back\"]').click()")
    wait("return document.querySelector('main article .entry-name')?.textContent==='toes'")
    js("document.querySelector('[aria-label=\"Forward\"]').click()")
    wait("return document.querySelector('main article .entry-name')?.textContent==='toenails'")
    for page in ['tag_group:ass','tag_group:fighting_games','tag_group:metatags','pool_groups']:
        search(page.replace('_',' '))
        selector='.search-outlines [data-name="'+page+'"]'
        click(selector+'>.wiki-row>.wiki-arrow')
        wait('return !!document.querySelector('+json.dumps(selector+'>.wiki-body>.wiki-tree>.wiki-branch')+')')
    js("[...document.querySelectorAll('nav button')].find(e=>e.textContent.includes('Favorites')).click()")
    wait("return !!document.querySelector('.favorite-card')")
    assert js("return !document.querySelector('.favorite-card details').open")
    click('.favorite-card summary')
    wait("return document.querySelector('.favorite-details').innerText.includes('Appearance')")
    screenshot('favorites-expanded.png')
    click('.favorite-card summary');screenshot('favorites-compact.png')
    assert api('favorites')[0]==saved
    assert api('job')['status']=='idle'
    assert not list(pathlib.Path(os.environ['XDG_DATA_HOME']).rglob('update.json'))
    print(json.dumps({'native_navigation':'PASS','checks':['tag labels open the main panel from sidebar, search and article lists','arrows expand without changing the main panel','repeated tag labels preserve expansion','back and forward navigation','main Body/Face/Hair source order','full Toes via label','Toenails via arrow and unique preview descendants','Toes search expansion and tag filter','star with full browsing path','compact and expanded favorites','outside Visual categories','cycle stopping','copy tag name','offline navigation']},indent=2))
finally:
    request('DELETE',prefix)
