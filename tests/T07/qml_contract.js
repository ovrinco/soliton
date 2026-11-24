const assert=require('assert');
let selected={id:'target',path:'/private/source',name:'target'},busy=false,error='',rows=[],preview=null,project='/private/project',target={currentText:'codex'},captured; let SystemSettings={request:(r,cb)=>{captured={r,cb}}};
    function pluginAction(action, section) {
        if (!selected || busy) return;
        var pluginId = selected.id;
        busy = true;
        SystemSettings.request({action: 'plugins', request: {action: action, id: pluginId, section: section}}, (result, failure) => {
            busy = false; error = failure;
            if (!failure) {
                rows = result.plugins;
                selected = rows.find(row => row.id === pluginId) || null;
                if (!selected || (action === 'enable' && !selected.enabled)
                    || (action === 'disable' && selected.enabled)
                    || (action === 'move' && selected.section !== section))
                    error = 'The plugin owner did not confirm the requested state. Review its current status.';
            }
        });
    }
    function skillAction(operation) {
        busy = true;
        SystemSettings.request({action: 'skills', request: {action: 'preview', operation: operation, project: project,
            paths: [selected.path], names: [selected.name], tools: [target.currentText]}}, (result, failure) => {
            busy = false; error = failure; if (!failure) preview = result;
        });
    }

pluginAction('disable','');assert.equal(captured.r.request.id,'target');captured.cb({plugins:[{id:'decoy',enabled:true},{id:'target',enabled:false}]},'');assert.equal(selected.id,'target');assert.equal(error,'');
pluginAction('enable','');captured.cb({plugins:[{id:'target',enabled:false}]},'');assert(error.includes('did not confirm'));
pluginAction('move','right');captured.cb({plugins:[{id:'target',enabled:true,section:'left'}]},'');assert(error.includes('did not confirm'));
pluginAction('disable','');captured.cb({},'owner refused');assert.equal(error,'owner refused');assert.equal(busy,false);
selected={id:'target',name:'target',path:'/private/source'};skillAction('share');assert.equal(captured.r.request.project,project);assert.deepEqual(captured.r.request.paths,['/private/source']);captured.cb({token:'private-token'},'');assert.equal(preview.token,'private-token');
console.log(JSON.stringify({status:'pass',checks:['target identity over decoy','enable mismatch','move mismatch','owner refusal','skill request contract'],limits:'extracted QML functions; mocked SystemSettings transport'}));
