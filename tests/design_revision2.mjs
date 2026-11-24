#!/usr/bin/env node
// Executes actual presentation functions with explicit JS stand-ins. NOT a Qt runtime test.
import fs from 'node:fs';import path from 'node:path';import vm from 'node:vm';import assert from 'node:assert/strict';import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const read=p=>fs.readFileSync(path.join(root,p),'utf8');let tests=0,checks=0;
const ok=(v,m)=>{assert.ok(v,m);checks++},eq=(a,b)=>{assert.deepEqual(JSON.parse(JSON.stringify(a)),JSON.parse(JSON.stringify(b)));checks++};
const test=(n,f)=>{f();tests++;console.log('PASS '+n)};
const presentation=vm.createContext({});vm.runInContext(read('ui/Presentation.js').replace(/^\.pragma.*$/mg,''),presentation);
function block(source,marker){const start=source.indexOf(marker);assert(start>=0,marker);let open=source.indexOf('{',start),i=open+1,depth=1,q='',escape=false,line=false,comment=false;for(;i<source.length&&depth;i++){let c=source[i],n=source[i+1];if(line){if(c==='\n')line=false;continue}if(comment){if(c==='*'&&n==='/'){comment=false;i++}continue}if(q){if(escape)escape=false;else if(c==='\\')escape=true;else if(c===q)q='';continue}if(c==='/'&&n==='/'){line=true;i++;continue}if(c==='/'&&n==='*'){comment=true;i++;continue}if(c==='"'||c==="'"||c==='`'){q=c;continue}if(c==='{')depth++;if(c==='}')depth--}assert.equal(depth,0);return source.slice(open+1,i-1)}
function fn(source,name){return `function ${name}(${source.slice(source.indexOf('function '+name+'(')+name.length+10).split(')')[0]}) {${block(source,'function '+name+'(')}}`}
// Explicit declaration reconstruction avoids relying on the helper's argument parser.
function named(source,name){let st=source.indexOf('function '+name+'('),open=source.indexOf('{',st);return source.slice(st,open+1)+block(source,'function '+name+'(')+'}'}
test('all 128 category subsets, seven independent toggles each, immutable input',()=>{
 const ids=Array.from({length:7},(_,i)=>'category-'+i);
 for(let mask=0;mask<128;mask++){let initial=ids.filter((x,i)=>mask&(1<<i));for(let id of ids){let copy=initial.slice(),out=presentation.toggled(initial,id);eq(initial,copy);eq(out.includes(id),!copy.includes(id));for(let other of ids.filter(x=>x!==id))eq(out.includes(other),copy.includes(other));eq(presentation.toggled(out,id).slice().sort(),initial.slice().sort());}}
});
test('saved category migration preserves subsets and rejects malformed values',()=>{
 eq(presentation.categorySelection(['a','b','a',null,4,''], 'old'),['a','b']);eq(presentation.categorySelection([], 'old'),[]);eq(presentation.categorySelection(undefined,'old'),['old']);eq(presentation.categorySelection(undefined,''),[]);
});
test('record routes normalize without nested History or Sources tabs',()=>{
 const r=vm.createContext({});vm.runInContext(read('ui/Routes.js').replace(/^\.pragma.*$/mg,''),r);
 for(let [tab,kind] of [['Chats','chat'],['Voice notes','voice_note'],['Runs','run']]){eq(presentation.historyKind(tab),kind);eq(presentation.historyTab(kind),tab);eq(r.normalize('Strategy',tab),{page:'Strategy',view:tab})}
 ok(read('ui/StrategyView.qml').includes("['Overview', 'Ideas', 'Execution', 'Tracker', 'Chats', 'Voice notes', 'Runs', 'Modules']"));
});
test('report missing and explicitly unknown counts are never synthetic zero',()=>{
 eq(presentation.reportKnown({},'added'),null);eq(presentation.reportKnown({summary:{added:0}},'added'),0);eq(presentation.reportKnown({summary:{added:34},summary_known:{added:false}},'added'),null);eq(presentation.reportKnown({summary:{added:null}},'added'),null);
});
test('actual navigation ordering gives each page its own filter scope and preserves calendar horizons',()=>{
 const routes=vm.createContext({});vm.runInContext(read('ui/Routes.js').replace(/^\.pragma.*$/mg,''),routes);
 let currentPage='Tasks';
 const c={Routes:routes,page:'Tasks',view:'Tasks',branches:['g1'],search:'task-only',priority:'high',status:'todo',showArchived:true,routeViews:{Tasks:'Tasks',Dashboard:'Week'},routeContexts:{},selectedId:'',activeKey:'',detailsOpen:false,detailFull:false,board:false,goalList:false,selectedDate:'2026-09-27',calendarMode:'Week',restoringRoute:false,window:{markPageVisited(){}},routeLeaving(){},remember(){}};
 Object.defineProperty(c,'page',{configurable:true,get(){return currentPage},set(v){if(v!==currentPage){currentPage=v;c.pageChanged()}}});
 Object.defineProperty(c,'routeKey',{get(){return c.page+'/'+c.view}});
 c.storedRouteContexts=()=>({...c.routeContexts,[c.routeKey]:{branches:c.branches.slice(),search:c.search,priority:c.priority,status:c.status,showArchived:c.showArchived,selectedDate:c.selectedDate,calendarMode:c.calendarMode}});
 c.Backend=c;let ctx=vm.createContext(c);
 vm.runInContext(named(read('shell.qml'),'onPageChanged').replace('onPageChanged','pageChanged')+'\n'+named(read('ui/Backend.qml').slice(read('ui/Backend.qml').indexOf('function navigate(p, v)')),'navigate'),ctx);
 ctx.navigate('Notes','Notes');eq(ctx.search,'');eq(ctx.branches,[]);eq(ctx.priority,'all');eq(ctx.status,'all');eq(ctx.showArchived,false);
 ctx.search='notes-only';ctx.navigate('Tasks','Tasks');eq(ctx.search,'task-only');eq(ctx.branches,['g1']);
 ctx.navigate('Dashboard','Week');eq(ctx.search,'');ctx.branches=['calendar-goal'];ctx.search='calendar-only';ctx.navigate('Dashboard','Month');eq(ctx.branches,['calendar-goal']);eq(ctx.search,'calendar-only');
 ctx.navigate('Tasks','Tasks');eq(ctx.search,'task-only');ctx.navigate('Notes','Notes');eq(ctx.search,'notes-only');
});
test('save settlement migrates scroll keys without closing or toggling the sheet',()=>{
 const c={ownSave:true,retainedIdentity:'new:task',identity:'id-7:task',d:{id:'id-7',dirty:false},preview:false,detailScroll:{contentX:0,contentY:381},documentReader:{scroll:{contentX:0,contentY:620}},Backend:{prefs:{scrollPositions:{'unrelated':{x:0,y:99}}},setting(k,v){this.prefs[k]=v}}};
 let ctx=vm.createContext(c);vm.runInContext(named(read('ui/DetailPane.qml'),'settleOwnSave'),ctx);ctx.settleOwnSave();eq(ctx.retainedIdentity,'id-7:task');eq(ctx.Backend.prefs.scrollPositions['sheet:id-7:task'].y,381);eq(ctx.Backend.prefs.scrollPositions['document:id-7:task'].y,620);eq(ctx.Backend.prefs.scrollPositions.unrelated.y,99);eq(ctx.preview,false);eq(ctx.ownSave,false);
});
test('identity changes during save cancel close-on-save and do not carry another record scroll',()=>{
 let c={ownSave:true,identity:'other:task',retainedIdentity:'old:task',isDocument:false,d:{id:'other',kind:'task'},preview:false,closeAfterSave:true,Backend:{saving:true},scheduleDetail:{expanded:true},relations:{expanded:true},connected:{expanded:true},saveReview:{close(){}},discardReview:{close(){}},more:{close(){}},documentProperties:{close(){}}};
 const ctx=vm.createContext(c);vm.runInContext(block(read('ui/DetailPane.qml'),'onIdentityChanged:'),ctx);eq(ctx.ownSave,false);eq(ctx.closeAfterSave,false);eq(ctx.retainedIdentity,'other:task');eq(ctx.scheduleDetail.expanded,false);
});
test('record restoration does not remove a second record or a changed in-flight draft',()=>{
 const s=read('ui/RecordHistory.qml');ok(s.includes('!root.hasDirtyDraft'));ok(s.includes('serial !== root.request || id !== root.recordId'));ok(s.includes('JSON.stringify(Backend.activeDraft || {}) === draftAtRequest'));
});
test('late history replies are invalidated when switching record kinds',()=>{
 const c={contextReady:true,kind:'chat',switchingKind:false,kindContexts:{voice_note:{query:'voice',selectedId:'v1',selectedRecord:{id:'v1',kind:'voice_note'},position:12345}},Backend:{prefs:{},available:true},visible:false,requestSerial:5,rememberKind(){},stopAudio(){c.audioStopped=true},cancelAudioPreparation(){},sourceDialog:{close(){}},recordActions:{close(){}},nextRequest(){return ++c.requestSerial}};
 c.consumedNavigationSerial=0;
 const ctx=vm.createContext(c);vm.runInContext(named(read('ui/HistoryView.qml'),'hasExplicitNavigation'),ctx);vm.runInContext(named(read('ui/HistoryView.qml'),'activateKind'),ctx);ctx.activateKind('voice_note',false);eq(ctx.audioStopped,true);eq(ctx.kind,'voice_note');eq(ctx.query,'voice');eq(ctx.selectedId,'v1');eq(ctx.restorePlaybackPosition,12345);eq(ctx.listRequest,6);eq(ctx.detailRequest,7);eq(ctx.revisionRequest,8);eq(ctx.switchingKind,false);
});
test('single sheet source guards and source revision reachability',()=>{
 const d=read('ui/DetailPane.qml'),r=read('ui/DocumentReader.qml'),h=read('ui/HistoryView.qml');ok(!d.includes('StackLayout'));ok(!d.includes("'Back to tasks'"));ok(!d.includes("text: 'Revert'"));ok(d.includes('text !== (root.d.body'));ok(r.includes('text !== root.text'));ok(h.indexOf("Accessible.name: 'Transcript revision'")>h.indexOf('id: sourceDialog'));ok(!read('shell.qml').includes('id: workFilterPopup'));
});
console.log(JSON.stringify({status:'passed',tests,checks,scope:'Node evaluation and source checks only; NOT Qt runtime, input or frame verification'}));
