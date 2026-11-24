.pragma library
function parse(s){var p=s.substring(0,10).split('-');return new Date(Number(p[0]),Number(p[1])-1,Number(p[2]),12,0,0);}
function iso(d){return d.getFullYear()+'-'+String(d.getMonth()+1).padStart(2,'0')+'-'+String(d.getDate()).padStart(2,'0');}
function add(s,n){var d=parse(s);d.setDate(d.getDate()+n);return iso(d);}
function serial(s){var p=s.substring(0,10).split('-');return Math.floor(Date.UTC(Number(p[0]),Number(p[1])-1,Number(p[2]))/86400000);}
function minutes(s){return Number(s.substring(11,13))*60+Number(s.substring(14,16));}
function minuteInDay(s,day){return (serial(s)-serial(day))*1440+minutes(s);}
function week(s,start){var d=parse(s);return add(s,-((d.getDay()-start+7)%7));}
function monthDays(s,start){var d=parse(s);d.setDate(1);var first=week(iso(d),start);return Array.from({length:42},(_,i)=>add(first,i));}
function clock(n){n=((n%1440)+1440)%1440;return String(Math.floor(n/60)).padStart(2,'0')+':'+String(n%60).padStart(2,'0');}
function label(s,format){var d=parse(s);return format==='short'?d.toLocaleDateString(Qt.locale(),{day:'numeric',month:'short'}):d.toLocaleDateString();}
// Clip to the chosen civil planning day and pack overlapping intervals into
// connected groups. Every event remains represented; no four-event cap.
function layout(events,day,start,end){
    var visible=events.filter(e=>e.event_kind==='timed').map(e=>({event:e,start:Math.max(start,minuteInDay(e.local_start,day)),end:Math.min(end,minuteInDay(e.local_end,day))})).filter(e=>e.end>e.start).sort((a,b)=>a.start-b.start||b.end-a.end||a.event.event_key.localeCompare(b.event.event_key));
    var group=[],ends=[],until=-1;
    function finish(){group.forEach(e=>e.columns=ends.length);group=[];ends=[];}
    visible.forEach(e=>{if(e.start>=until){finish();until=e.end;}var lane=ends.findIndex(v=>v<=e.start);if(lane<0)lane=ends.length;ends[lane]=e.end;e.column=lane;group.push(e);until=Math.max(until,e.end);});finish();return visible;
}

// Completion remains visible for four seconds. The exact transition receipt
// stays in the existing workspace preferences for a short Undo after a view
// change or restart; the task itself is still written through its owner.
function receiptEntries(backend) {
    var receipt = backend.prefs.lastTaskCompletion || {};
    return receipt.entries || (receipt.id ? [receipt] : []);
}
function statusRecord(backend, record) {
    if (!record || !record.recurrence || record.recurrence === 'none' || record.occurrence) return record;
    return backend.todayEvents.find(e => e.id === record.id && e.occurrence && e.date === backend.today) || record;
}
function effectiveStatus(backend, record) { return (statusRecord(backend, record) || {}).status; }
function completion(backend, record, next, done, collect) {
    if (!record || !record.id) return;
    var recurring = record.recurrence && record.recurrence !== 'none';
    var occurrence = statusRecord(backend, record);
    var previous = occurrence.status;
    var currentDone = previous === 'completed';
    if (next === 'completed' && currentDone) { if (done) done(record, true); return; }
    if (next === 'todo' && !currentDone && previous === 'todo') { if (done) done(record, true); return; }
    function settled(value, ok) {
        if (ok && next === 'completed') {
            var now = Date.now();
            var entry = {
                id: record.id, previous: previous, afterHash: value.hash,
                occurrence: recurring && occurrence ? occurrence.occurrence || '' : '',
                occurrence_date: recurring && occurrence ? occurrence.occurrence_date || '' : ''
            };
            if (collect) collect(entry);
            else backend.setting('lastTaskCompletion', {entries:[entry], graceUntil:now + 4000, undoUntil:now + 30000});
        } else if (ok && next !== 'completed') {
            var old = backend.prefs.lastTaskCompletion || {}, kept = receiptEntries(backend).filter(e =>
                e.id !== record.id || (recurring && e.occurrence !== (occurrence.occurrence || '')));
            if (kept.length !== receiptEntries(backend).length)
                backend.setting('lastTaskCompletion', kept.length ? {entries:kept, graceUntil:old.graceUntil, undoUntil:old.undoUntil} : null);
        }
        if (done) done(value, ok);
    }
    backend.moveStatus(occurrence, next, settled);
}

function showTask(backend, record, now) {
    record = statusRecord(backend, record);
    if (backend.showCompleted || record.status !== 'completed') return true;
    var receipt = backend.prefs.lastTaskCompletion || {};
    return receipt.graceUntil > now && receiptEntries(backend).some(e =>
        e.id === record.id && (!record.occurrence || e.occurrence === record.occurrence));
}

function canUndo(backend, now) {
    var receipt = backend.prefs.lastTaskCompletion || {};
    return receipt.undoUntil > now && receiptEntries(backend).some(e => {
        var current = backend.get(e.id);
        return !!current && current.hash === e.afterHash;
    });
}

function undoCompletion(backend) {
    var receipt = backend.prefs.lastTaskCompletion || {};
    if (receipt.undoUntil <= Date.now()) {
        backend.message('This completion changed. Review the task before reopening it.', true);
        return;
    }
    var entries = receiptEntries(backend), failed = [], changed = 0, index = 0;
    function next() {
        if (index >= entries.length) {
            backend.setting('lastTaskCompletion', failed.length ? {entries:failed, graceUntil:receipt.graceUntil, undoUntil:receipt.undoUntil} : null);
            backend.message(changed + ' completion' + (changed === 1 ? '' : 's') + ' undone' + (failed.length ? ' · ' + failed.length + ' changed; review them' : ''), failed.length > 0);
            return;
        }
        var entry = entries[index++], current = backend.get(entry.id);
        if (!current || current.hash !== entry.afterHash) { failed.push(entry); next(); return; }
        function settled(value, ok) { if (ok) changed++; else failed.push(entry); next(); }
        if (entry.occurrence)
            backend.send('occurrence', {id: entry.id, hash: current.hash,
                occurrence: entry.occurrence, occurrence_date: entry.occurrence_date,
                status: entry.previous}, settled);
        else
            backend.mutate(current, {status: entry.previous}, settled);
    }
    next();
}
