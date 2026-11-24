import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import crypto from 'node:crypto';
const [candidate, output] = process.argv.slice(2);
fs.mkdirSync(output, {recursive:false});
const sources = Object.fromEntries(['HistoryView','Backend'].map(n=>[n,fs.readFileSync(path.join(candidate,'ui',n+'.qml'),'utf8')]));
function method(source, name) {
 const begin=source.indexOf('function '+name+'(');assert(begin>=0,name);
 let open=source.indexOf('{',begin), depth=1, quote='', escape=false, line=false, block=false;
 for(let i=open+1;i<source.length;i++) {
  const c=source[i],n=source[i+1];
  if(line){if(c==='\n')line=false;continue;}if(block){if(c==='*'&&n==='/'){block=false;i++;}continue;}
  if(quote){if(escape)escape=false;else if(c==='\\')escape=true;else if(c===quote)quote='';continue;}
  if(c==='/'&&n==='/'){line=true;i++;continue;}if(c==='/'&&n==='*'){block=true;i++;continue;}
  if(c==='"'||c==="'"||c==='`'){quote=c;continue;}
  if(c==='{')depth++;if(c==='}'&&--depth===0)return source.slice(begin,i+1);
 }
 throw Error('Unterminated '+name);
}
function fixture() {
 const pending=[],later=[];
 const root={contextReady:true, visible:true, kind:'all', switchingKind:false, consumedNavigationSerial:0,
  kindContexts:{},query:'',offset:0,items:[],total:0,selectedId:'',selectedRecord:null,detailFocused:false,
  editingSegment:false,resolvingNavigationKind:false,requestSerial:0,revisions:[],audioPlaybackSource:'',errorText:'',
  persistCorrectionDraft(){},stopAudio(){},cancelAudioPreparation(){},sourceDialog:{close(){}},recordActions:{close(){}},
  loadList(){},restoreCorrectionDraft(){},loadRevisions(){},recordKindResolved(kind){this.activateKind(kind);Backend.view=kind==='voice_note'?'Voice notes':'Chats';},
 };
 for(const match of sources.HistoryView.matchAll(/property\s+(?:\w+\s+)?(\w+)\s*:/g))if(!(match[1] in root))root[match[1]]=null;
 const Backend={available:true,page:'Tasks',view:'List',prefs:{recordViews:{chat:{selectedId:'missing-chat'},voice_note:{selectedId:'wrong-kind'}}},historySelectedId:'',historyNavigation:{serial:0,id:''},drafts:{task:{dirty:true,text:'keep draft'}},
 setting(k,v){this.prefs[k]=v},remember(){},navigate(p,v){this.page=p;this.view=v},send(command,args,cb){pending.push({command,args,cb})}};
 const context=vm.createContext({root,Backend,audioPlayer:{position:0},detailScrollRestoreTimer:{restart(){}},Qt:{callLater(fn){later.push(fn)}},Object,Number,String});
 for(const name of ['rememberKind','activateKind','nextRequest','syncToBackend','syncFromBackend','selectRecord','hasExplicitNavigation']) {
  if(sources.HistoryView.includes('function '+name+'('))root[name]=vm.runInContext('(function(){with(root){return '+method(sources.HistoryView,name)+'}})()',context);
 }
 const showHistory=vm.runInContext('(function(){with(Backend){return '+method(sources.Backend,'showHistory')+'}})()',context);
 function reply(id,kind='voice_note',ok=true){const entry=pending.findLast(p=>p.command==='history-detail'&&p.args.id===id);assert(entry,'request '+id);entry.cb(ok?{id,kind,revision:1,source_refs:[{path:'canonical/'+id}]}:null,ok,ok?'':'History record not found');while(later.length)later.shift()();}
 return {root,Backend,pending,reply,show:id=>showHistory(id)};
}
const results=[];
for(const [name,test] of [
 ['explicit ID wins on fresh mount over stale chat preference',()=>{const f=fixture();f.show('voice-A');f.root.activateKind('chat',true);assert.equal(f.root.selectedId,'voice-A');f.reply('voice-A');assert.equal(f.root.kind,'voice_note');assert.equal(f.root.selectedRecord.id,'voice-A');assert.equal(f.root.selectedRecord.source_refs[0].path,'canonical/voice-A');assert.equal(f.Backend.view,'Voice notes');assert.notEqual(f.Backend.prefs.recordViews.chat.selectedId,'voice-A');assert.equal(f.Backend.drafts.task.text,'keep draft');}],
 ['new explicit request wins over out-of-order detail',()=>{const f=fixture();f.show('voice-A');f.root.activateKind('chat',true);f.show('voice-B');f.root.syncFromBackend();f.reply('voice-B');f.reply('voice-A');assert.equal(f.root.selectedId,'voice-B');assert.equal(f.root.selectedRecord.id,'voice-B');}],
 ['missing explicit ID keeps error local and preserves unrelated draft',()=>{const f=fixture();f.show('missing');f.root.activateKind('chat',true);f.reply('missing','chat',false);assert.equal(f.root.selectedId,'missing');assert.equal(f.root.errorText,'History record not found');assert.equal(f.Backend.drafts.task.text,'keep draft');assert.equal(f.Backend.page,'Strategy');}],
 ['ordinary kind return restores valid remembered selection',()=>{const f=fixture();f.Backend.prefs.recordViews.chat.selectedId='valid-chat';f.root.activateKind('chat',true);assert.equal(f.root.selectedId,'valid-chat');f.reply('valid-chat','chat');f.root.activateKind('voice_note');f.root.activateKind('chat');assert.equal(f.root.selectedId,'valid-chat');assert.equal(f.root.selectedRecord.kind,'chat');}],
]) {try{test();results.push({name,result:'PASS'});}catch(e){results.push({name,result:'FAIL',error:e.stack});}}
const receipt={candidate:path.resolve(candidate),source_sha256:Object.fromEntries(Object.entries(sources).map(([n,s])=>['ui/'+n+'.qml',crypto.createHash('sha256').update(s).digest('hex')])),scope:'Extracted actual QML navigation methods; synthetic asynchronous transport, no owner/native acceptance',results,exit_code:results.some(r=>r.result==='FAIL')?1:0};
fs.writeFileSync(path.join(output,'result.json'),JSON.stringify(receipt,null,2)+'\n');console.log(JSON.stringify(results,null,2));process.exitCode=receipt.exit_code;
