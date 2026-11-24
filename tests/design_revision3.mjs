#!/usr/bin/env node
// Executed presentation/state tests. Not Qt type checking or native observations.
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
const W=vm.createContext({});vm.runInContext(read('ui/Workplane.js').replace(/^\.pragma.*$/mg,''),W);
const tests=[];
function test(name,fn){fn();tests.push(name);console.log('PASS '+name);}
function block(s,marker){let a=s.indexOf(marker);assert(a>=0,marker);let i=s.indexOf('{',a)+1,start=i,depth=1,q='',esc=false,line=false,comment=false;for(;i<s.length;i++){let c=s[i],n=s[i+1];if(line){if(c==='\n')line=false;continue}if(comment){if(c==='*'&&n==='/'){comment=false;i++}continue}if(q){if(esc)esc=false;else if(c==='\\')esc=true;else if(c===q)q='';continue}if(c==='/'&&n==='/'){line=true;i++;continue}if(c==='/'&&n==='*'){comment=true;i++;continue}if('"\'`'.includes(c)){q=c;continue}if(c==='{')depth++;if(c==='}'&&!--depth)return s.slice(start,i)}throw Error('Unbalanced '+marker)}
function named(s,name){let a=s.indexOf('function '+name+'('),b=s.indexOf('{',a);assert(a>=0);return s.slice(a,b+1)+block(s,'function '+name+'(')+'}'}
const eq=(a,b)=>assert.deepEqual(JSON.parse(JSON.stringify(a)),JSON.parse(JSON.stringify(b)));
test('overflow: empty, exact fit, invalid, real overflow and AlwaysOff',()=>{
 for(let x of [0,1,1.1,NaN,Infinity,-1])assert.equal(W.overflows(x,0,2),false);
 for(let x of [.02,.5,.999])assert.equal(W.overflows(x,0,2),true);
 assert.equal(W.overflows(.3,2,2),false);
 assert.match(read('ui/SScrollBar.qml'),/visible: control.hasOverflow/);
});
test('long multiline collection summaries never alter original source strings',()=>{
 let original='a\r\n\n<b>🙂</b>\t\u2003long',snap=original;
 assert.equal(W.previewLine(original),'a <b>🙂</b> long');assert.equal(original,snap);
 assert.equal(W.previewLine(null),'');assert.equal(W.previewLine(12),'12');
 let s=read('ui/CollectionRow.qml');assert.equal((s.match(/maximumLineCount: 1/g)||[]).length,3);assert.match(s,/clip: true/);
});
test('literal replacement handles metacharacters, dollars and Unicode offsets',()=>{
 let text='🙂 [a] İ foo [a] $1';let m=W.literalMatches(text,'[a]');eq(m,[{start:3,length:3},{start:13,length:3}]);
 assert.equal(W.replaceMatches(text,m,'$&🙂'),'🙂 $&🙂 İ foo $&🙂 $1');
 assert.equal(W.replaceMatches('one ONE One',W.literalMatches('one ONE One','one'),''),'  ');
 assert.equal(W.literalMatches('abc','').length,0);assert.equal(W.literalMatches('a.*b','.*').length,1);
});
test('popup anchors remain beside the trigger and flip above at the lower edge',()=>{
 eq(W.popupPosition({x:800,y:50,width:34,height:36},{width:238,height:180},{width:1000,height:700}),{x:596,y:90});
 eq(W.popupPosition({x:980,y:660,width:34,height:36},{width:238,height:180},{width:1000,height:700}),{x:754,y:476});
 for(let w of [320,640,1440]){let p=W.popupPosition({x:-50,y:-50,width:34,height:36},{width:238,height:180},{width:w,height:480});assert(p.x>=8&&p.x+238<=w-8);assert(p.y>=8);}
 assert.match(read('ui/DetailPane.qml'),/Workplane.popupPosition/);assert.match(read('ui/GoalsView.qml'),/Workplane.popupPosition/);
});
test('wheel mapping: high-resolution pixels, fractional notches, sign, origin, bounds',()=>{
 assert.equal(W.wheelPosition(200,0,2000,500,12,120,3,16),188);
 assert.equal(W.wheelPosition(200,0,2000,500,0,-120,3,16),248);
 assert.equal(W.wheelPosition(200,0,2000,500,0,-15,3,16),206);
 assert.equal(W.wheelPosition(200,0,2000,500,0,120,3,16),152);
 assert.equal(W.wheelPosition(100,100,300,500,0,-120,3,16),100);
 assert.equal(W.wheelPosition(1400,100,2000,500,0,-12000,3,16),1600);
 const s=read('ui/SCollectionWheel.qml');assert.match(s,/PointerDevice.Mouse/);assert.match(s,/Qt.NoModifier/);assert.match(s,/event.accepted = false/);assert(!s.includes('Behavior'));
});
test('idle Select exits on any route departure; in-flight work is not canceled',()=>{
 const s=read('ui/TaskView.qml');let ctx={selectionMode:false,selectionScope:'',selectionRouteLeft:false,scopeKey:'same',batchBusy:false,selectedTaskIds:['a','b'],lastSelectedId:'a',forceActiveFocus(){},deselectAll(){this.selectedTaskIds=[];this.lastSelectedId='';}};ctx.root=ctx;
 const c=vm.createContext(ctx);vm.runInContext(named(s,'deselectAll')+'\n'+named(s,'beginSelection')+'\n'+named(s,'endSelection')+'\n'+named(s,'onRouteLeaving'),c);
 c.beginSelection();assert(c.selectionMode);c.onRouteLeaving();assert.equal(c.selectionMode,false);eq(c.selectedTaskIds,[]);
 c.beginSelection();c.batchBusy=true;c.selectedTaskIds=['frozen-id'];c.onRouteLeaving();assert(c.selectionMode);assert(c.selectionRouteLeft);eq(c.selectedTaskIds,['frozen-id']);
 c.batchBusy=false;c.endSelection();assert.equal(c.selectionMode,false);
 assert(s.includes('onBatchBusyChanged'));assert(s.includes('selectedTaskIds.slice()'));
});
test('both task layouts use the protected compact TaskCard and one three-state control',()=>{
 let s=read('ui/TaskView.qml');assert.equal((s.match(/compact: true/g)||[]).length,2);
 let t=read('ui/TaskCard.qml');assert.match(t,/implicitHeight: compact \? 62 : 100/);
 for (const state of ['todo','in_progress','completed']) assert(t.includes(`Cal.completion(Backend, card.record, '${state}')`));
 assert.equal((t.match(/id: statusButton/g)||[]).length,1);
 assert(t.includes('Layout.preferredWidth: 32'));assert(t.includes('Layout.preferredHeight: 32'));
 assert(t.includes('visible: !card.selectable'));assert(t.includes('visible: card.selectable'));
});
test('navigation has one effective collapse authority and constant icon alignment',()=>{
 const s=read('shell.qml'),n=read('ui/SNavItem.qml');assert(!s.includes('Backend.navCollapsed || surface.width'));
 const rail=read('ui/ShellRail.qml');assert(s.includes('Layout.preferredWidth: railWidth'));assert(rail.includes('collapsed ? Theme.railFoldedWidth : Theme.railWidth'));assert(rail.includes("objectName: 'navigation-toggle'"));assert.match(n,/property int iconX: 11/);assert.match(n,/x: control.iconX/);
 assert(!n.includes('width: 2; height: parent.height-16'));assert(!s.includes('PageHeading {'));
});
test('Goals negotiates bounded categories and keeps its search inside the goal pane',()=>{
 let s=read('ui/GoalsView.qml');assert(s.includes('SplitView.maximumWidth: Math.max(136, Math.min(280, root.width * 0.40))'));
 for(let width of [320,414,600,941,1440]){let max=Math.max(136,Math.min(280,width*.4));assert(max<=width/2);assert(width-max-10>=174);}
 assert(s.indexOf("scopeName: 'goals'")>s.indexOf("objectName: 'goals-pane'"));assert(s.includes('Layout.minimumWidth:0;Layout.preferredWidth:'));
});
test('keyboard rows have no reference line and all six rows fit their width by weight()',()=>{
 let s=read('ui/SettingsKeyboard.qml');assert(s.includes('model: root.physicalRows'));assert(!s.includes("root.layout === 'us' ? root.physicalRows : []"));
 let data=s.match(/readonly property var physicalRows: (\[[\s\S]*?\n    \])/)[1];let c=vm.createContext({});vm.runInContext('var rows='+data+';'+named(s,'weight'),c);
 for(let width of [640,920])for(let row of c.rows){let total=row.reduce((n,k)=>n+c.weight(k),0);let sum=row.reduce((n,k)=>n+(width-(row.length-1)*3)*c.weight(k)/total,0)+(row.length-1)*3;assert(Math.abs(sum-width)<.00001);}
 assert.doesNotMatch(read('ui/SettingsDevicePane.qml'),/US keyboard reference/);
});
test('Settings inventories are direct peers, retain old-link compatibility and owner guards',()=>{
 let s=read('ui/SettingsView.qml');assert(s.includes("['General', 'Activity', 'Monitors', 'Key bindings', 'Plugins', 'Skills', 'Zeno']"));assert(s.includes("Backend.view === 'Plugins & Skills' ? 'Plugins'"));assert(!s.includes('skillView'));
 let inv=read('ui/SettingsInventory.qml');assert(!inv.includes("action: 'apply'"));assert(inv.includes('PluginsPage {'));assert(inv.includes('SkillsPage {'));
});
test('voice duration is not the selected passage time; copy does not expose normalization',()=>{
 let s=read('ui/HistoryView.qml');assert(!s.includes("'Voice level'"));assert(!s.includes("'Preparing voice level…'"));assert(!s.includes("text: 'Correct"));assert(s.includes('Number(root.selectedRecord.duration_ms) > 0'));
 let voice=block(s,'TranscriptReader {\n                        id: transcriptReader');assert(voice.includes("title: 'Transcript: '"));assert(!voice.includes('formatDuration'));
 assert(s.includes('expected_revision: root.correctionRevision'));
});
test('Replace is editable PlainText-only, identity-pinned and remains a draft operation',()=>{
 let s=read('ui/DocumentReader.qml');let f=named(s,'replaceText');assert(f.includes('textFormat !== TextEdit.PlainText'));assert(f.includes('replaceIdentity !== positionKey'));assert(f.includes('root.edited(after)'));assert(!f.includes('Backend.send'));
 let d=read('ui/DetailPane.qml');assert(d.includes('id: descriptionPreview'));assert(d.includes('textFormat: TextEdit.MarkdownText'));assert(d.includes('textFormat: TextEdit.PlainText'));assert(d.includes('Math.max(120, description.implicitHeight, descriptionPreview.implicitHeight)'));assert(read('ui/Choice.qml').includes('control.textFor(row.modelData)'));assert(d.includes("{label:'In progress', value:'in_progress'}"));assert(d.includes('!root.descriptionRead && text !=='));
});
test('reading and scope preservation remain in the implementation',()=>{
 assert.match(read('ui/RetainedScroll.qml'),/interval:650/);assert.match(read('ui/RetainedScroll.qml'),/onRouteLeaving\(\).*capture\(\)/);
 assert(read('ui/HistoryView.qml').indexOf('id: searchField')>read('ui/HistoryView.qml').indexOf("objectName: 'history-list-pane'"));
 assert(read('ui/TaskView.qml').includes("text: 'Board'; active: Backend.board"));
});
test('supporting splitters use the same quiet handle, including console',()=>{
 for(let p of ['HistoryView','InboxView','LearningFeed','StackMap','Dashboard','GoalsView'])assert(read('ui/'+p+'.qml').includes('SSplitHandle {'),p);
 assert(read('ui/SettingsInventory.qml').includes('Loader'));assert(!read('ui/SettingsInventory.qml').includes('SplitView'));assert(read('console/archive/ConsolePane.qml').includes('Planning.SSplitHandle'));
 let s=read('ui/SSplitHandle.qml');assert(s.includes('color: "transparent"'));assert(s.includes('hovered || handle.SplitHandle.pressed'));
});
console.log(JSON.stringify({status:'passed',tests:tests.length,scope:'actual JS functions + explicit Node state shims + static source contracts; NOT native geometry/Qt execution',cases:tests},null,2));
