import assert from 'node:assert/strict';
import {test} from 'node:test';
import ts from 'typescript';
import fs from 'node:fs';
const js=ts.transpileModule(fs.readFileSync(new URL('../src/navigation.ts',import.meta.url),'utf8'),{compilerOptions:{module:ts.ModuleKind.ESNext,target:ts.ScriptTarget.ES2022}}).outputText;
const {mergePreview,loadsWiki}=await import('data:text/javascript;base64,'+Buffer.from(js).toString('base64'));
const e=(name,entries=[],kind='tag')=>({name,entries,kind,display:name,description:'',note:''});
test('categories keep explicit lists; real tags always load their own pages',()=>{
 assert.equal(loadsWiki(e('body',[e('ass')],'group')),false);
 assert.equal(loadsWiki(e('feet',[],'group')),true);
 assert.equal(loadsWiki(e('toes',[e('ninja_toes')])),true);
 assert.equal(loadsWiki(e('Appearance',[],'section')),false);
});
test('unique parent descendants survive without duplicating existing tags or mutating cache',()=>{
 const full=[e('Related',[e('Appearance',[e('toenails'),e('long_toes')],'section')],'section')];
 const preview=[e('toenails',[e('sharp_toenails')]),e('long_toes'),e('ninja_toes')];
 const merged=mergePreview(full,preview);
 assert.equal(merged[0].entries[0].entries[0].entries[0].name,'sharp_toenails');
 assert.equal(merged[1].name,'ninja_toes');assert.equal(merged.length,2);
 assert.equal(full[0].entries[0].entries[0].entries.length,0);
 assert.deepEqual(mergePreview(merged,preview),merged);
});
