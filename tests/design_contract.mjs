#!/usr/bin/env node
/** Dependency-free presentation tests. Node != Qt/QML runtime. */
import fs from 'node:fs';
import path from 'node:path';
import vm from 'node:vm';
import assert from 'node:assert/strict';
import {fileURLToPath} from 'node:url';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'..');
const read=p=>fs.readFileSync(path.join(root,p),'utf8');
let tests=0, assertions=0;
const ok=(v,message)=>{assert.ok(v,message);assertions++;};
const eq=(a,b)=>{assert.deepEqual(a,b);assertions++;};
const test=(name,fn)=>{fn();tests++;console.log('PASS '+name);};
const context=vm.createContext({});vm.runInContext(read('ui/TranscriptDocument.js').replace(/^\.pragma.*$/mg,''),context);
const build=x=>JSON.parse(JSON.stringify(context.build(x)));
const target=(d,a,b)=>{const x=context.target(d,a,b);return x?JSON.parse(JSON.stringify(x)):null;};
test('voice characters and UTF-16 segment mapping',()=>{
 const raw='  A😀 B\r\nC  ',d=build([{id:'voice/a',text:raw,start_ms:500,end_ms:3500}]);
 eq(d.text,'A😀 B\nC');
 for(let i=0;i<d.text.length;i++) {const t=target(d,i,i+1);eq(t.id,'voice/a');eq(raw.slice(t.sourceStart,t.sourceEnd).replace(/\r\n?/g,'\n'),d.text.slice(i,i+1));}
 eq(target(d,d.text.length,d.text.length).sourceStart,raw.trimEnd().length);
});
test('synthetic separators and cross-segment writes refuse a target',()=>{
 const d=build([{id:'a',text:'One.'},{id:'b',text:'Two.'}]);eq(d.text,'One. Two.');
 eq(target(d,4,4),null);eq(target(d,0,7),null);eq(target(d,5,9).id,'b');
 eq(target(d,9,5).id,'b');eq(target(d,-1,0),null);eq(target(d,99,99),null);
});
test('paragraphs, speakers and whitespace-only inputs',()=>{
 const d=build([{id:'a',text:'a'.repeat(445)+'.',speaker:'One'},{id:'b',text:'B.',speaker:'One'},{id:'c',text:'C.',speaker:'Two'}]);
 ok(d.text.includes('.\n\nB.\n\nC.'));eq(d.spans[2].speaker,'Two');
 eq(build([{id:'blank',text:'  \r\n  '}]).text,'');eq(target(build([]),0,0),null);
 eq(context.atTime(build([{id:'a',text:'A',start_ms:500,end_ms:1000},{id:'b',text:'B',start_ms:1000}]),1100).id,'b');
});
test('500 deterministic Unicode/CRLF source-offset cases',()=>{
 const pieces=['word','😀',' é ','\r\n','\n','漢字','e\u0301','—'];let seed=9137;
 const rnd=n=>{seed=(Math.imul(seed,1664525)+1013904223)>>>0;return seed%n;};
 for(let n=0;n<500;n++) {
  let raw=' ';for(let j=0;j<9;j++)raw+=pieces[rnd(pieces.length)];raw+=' ';
  const d=build([{id:'case/'+n,text:raw}]);
  eq(d.text,raw.trim().replace(/\r\n?/g,'\n'));
  for(let p=0;p<d.text.length;p++){const t=target(d,p,p+1);eq(raw.slice(t.sourceStart,t.sourceEnd).replace(/\r\n?/g,'\n'),d.text.slice(p,p+1));}
 }
});
test('reader Find uses original Unicode offsets and literal queries',()=>{
 const source=read('ui/DocumentReader.qml');
 const fn=source.slice(source.indexOf('    function findNext('),source.indexOf('    function copyText('));
 const editor={value:'İstanbul. AFTER 😀 literal [x] AFTER',selectionStart:0,selectionEnd:0,y:0,
  get length(){return this.value.length;},getText(){return this.value;},
  select(a,b){this.selectionStart=a;this.selectionEnd=b;},positionToRectangle(){return {y:0};}};
 const c=vm.createContext({editor,findField:{text:'after'},findStatus:'',viewport:{contentY:0,contentHeight:1000,height:300}});
 vm.runInContext(fn,c);c.findNext(false);eq(editor.selectionStart,10);eq(editor.selectionEnd,15);
 c.findNext(false);eq(editor.selectionStart,31);c.findNext(true);eq(editor.selectionStart,10);
 c.findField.text='[x]';c.findNext(false);eq(editor.getText().slice(editor.selectionStart,editor.selectionEnd),'[x]');
 c.findField.text='.';c.findNext(false);eq(editor.selectionStart,8);
 c.findField.text='not present';c.findNext(false);eq(c.findStatus,'Not found');
});
// Run actual Theme functions with a deterministic color-only Qt shim.
function color(v){if(typeof v==='object')return v;if(v==='transparent')return {r:0,g:0,b:0,a:0};let h=v.replace('#','');return {r:parseInt(h.slice(0,2),16)/255,g:parseInt(h.slice(2,4),16)/255,b:parseInt(h.slice(4,6),16)/255,a:1};}
const Qt={rgba:(r,g,b,a)=>({r,g,b,a}),tint:(a,b)=>{a=color(a);b=color(b);return {r:a.r*(1-b.a)+b.r*b.a,g:a.g*(1-b.a)+b.g*b.a,b:a.b*(1-b.a)+b.b*b.a,a:1};}};
function functions(s){let result='';const re=/\bfunction\s+\w+\s*\([^)]*\)\s*\{/g;for(let m; m=re.exec(s);){if(m[0].includes('function apply(')){re.lastIndex=s.indexOf('    function channel',m.index);continue;}let i=re.lastIndex,depth=1,q='',escaped=false;for(;i<s.length&&depth;i++){const c=s[i];if(q){if(escaped)escaped=false;else if(c==='\\')escaped=true;else if(c===q)q='';}else if(c==='"'||c==="'"||c==='`')q=c;else if(c==='{')depth++;else if(c==='}')depth--;}
result+=s.slice(m.index,i)+'\n';re.lastIndex=i;}return result;}
const theme=vm.createContext({Qt,base:color('#121214'),text:color('#e3e1dc')});vm.runInContext(functions(read('ui/Theme.qml')),theme);
test('semantic text >=4.5:1 on observed dark and review light surfaces',()=>{
 for(const palette of [ ['#121214','#e3e1dc',['#121214','#0b0b0d','#1f1f24','#33301f']],['#faf8f3','#202126',['#faf8f3','#ffffff','#eeeae1','#ece5cf']] ]){
  theme.base=color(palette[0]);theme.text=color(palette[1]);
  const surfaces=palette[2].map(color);
  for(const c of ['#8e8e93','#d15f4e','#7f8f6a']){
   const ink=theme.readableAcross(c,surfaces);
   for(const bg of surfaces)ok(theme.contrast(bg,ink)>=4.5,JSON.stringify({c,bg,ink}));
  }
  ok(theme.contrast(color('#d9a93c'),theme.foregroundFor(color('#d9a93c')))>=4.5);
 }
});
const cal=vm.createContext({Date});vm.runInContext(read('ui/CalendarMath.js').replace(/^\.pragma.*$/mg,''),cal);
test('protected calendar arithmetic: rollover, exclusive midnight and Sunday origin',()=>{
 eq(cal.minuteInDay('2026-09-22T01:00','2026-09-21'),1500);eq(cal.clock(1500),'01:00');eq(cal.add('2026-09-21',1),'2026-09-22');
 const events=[{event_key:'midnight',event_kind:'timed',local_start:'2026-09-21T23:00',local_end:'2026-09-22T00:00'}];
 eq(cal.layout(events,'2026-09-21',840,1920).length,1);eq(cal.layout(events,'2026-09-22',0,480).length,0);
 eq(cal.monthDays('2026-09-01',0)[0],'2026-08-30');eq(cal.week('2026-09-20',0),'2026-09-20');
});
test('retained-scroll store/capture isolates identities and restore windows',()=>{
 const writes=[];const x={ready:true,active:true,positionKey:'A',observedKey:'A',savedX:0,savedY:90,flickable:{contentX:0,contentY:90},restoreTimer:{running:false,restart(){this.running=true;}},saveTimer:{restart(){},stop(){}},Backend:{restored:true,restoringRoute:false,prefs:{scrollPositions:{}},setting(k,v){this.prefs[k]=v;writes.push(k);}}};
 const ctx=vm.createContext(x);vm.runInContext(functions(read('ui/RetainedScroll.qml')),ctx);ctx.capture();eq(ctx.Backend.prefs.scrollPositions.A.y,90);
 ctx.positionKey='B';ctx.flickable.contentY=0;ctx.capture();eq(ctx.Backend.prefs.scrollPositions.A.y,90);ok(!ctx.Backend.prefs.scrollPositions.B);
 ctx.observedKey='B';ctx.restoreTimer.running=true;ctx.capture();ok(!ctx.Backend.prefs.scrollPositions.B);
});
console.log(JSON.stringify({scope:'Node execution of presentation JS and color/coordinate shims; NOT native QML',tests,assertions,status:'passed'}));
