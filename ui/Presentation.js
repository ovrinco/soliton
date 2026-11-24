.pragma library

// Presentation-only transformations. No record writes or owner assumptions.
function toggled(values, id) {
    var next = Array.isArray(values) ? values.slice() : [];
    var at = next.indexOf(id);
    if (at < 0) next.push(id); else next.splice(at, 1);
    return next;
}
function categorySelection(saved, legacy) {
    if (Array.isArray(saved)) return saved.filter(function(id, i, all) { return typeof id === 'string' && id && all.indexOf(id) === i; });
    return typeof legacy === 'string' && legacy ? [legacy] : [];
}
function matchesText(record, query) {
    return !query || [record.title, record.body, record.category_title, record.goal_title, (record.tags || []).join(' ')].join(' ').toLowerCase().indexOf(String(query).toLowerCase()) >= 0;
}
function historyKind(tab) {
    return tab === 'Voice notes' ? 'voice_note' : tab === 'Runs' ? 'run' : 'chat';
}
function historyTab(kind) {
    return kind === 'voice_note' ? 'Voice notes' : kind === 'run' ? 'Runs' : 'Chats';
}
function reportKnown(report, key) {
    var known = report.summary_known || {};
    var value = (report.summary || {})[key];
    return known[key] === false || value === undefined || value === null ? null : value;
}
function reportGroupKey(group, index) {
    return String(group.id || [group.day, group.action, group.category, group.actor, index].join('|'));
}
