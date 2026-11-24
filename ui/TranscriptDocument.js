.pragma library

// Presentation only. Offsets are UTF-16, matching QML TextEdit positions.
// Every visible source character maps to the original segment and offset.
// Inserted paragraph whitespace has no source offset and is never a write target.
function build(segments) {
    var text = '', spans = [], paragraphLength = 0, lastSpeaker = '';
    (segments || []).forEach(function(segment, index) {
        var raw = String(segment.text || ''), first = raw.search(/\S/);
        if (first < 0) return;
        var last = raw.length;
        while (last > first && /\s/.test(raw.charAt(last - 1))) --last;
        var speaker = String(segment.speaker || '');
        if (text.length) {
            var paragraph = (lastSpeaker && speaker && lastSpeaker !== speaker) ||
                (paragraphLength >= 440 && /[.!?…。！？]["'”’)]?$/.test(text));
            var separator = paragraph ? '\n\n' : ' ';
            text += separator;
            paragraphLength = paragraph ? 0 : paragraphLength + 1;
        }
        var start = text.length, offsets = [], displayed = '';
        for (var i = first; i < last; ++i) {
            var ch = raw.charAt(i);
            // QTextDocument normalizes CRLF. Canonicalize explicitly and retain
            // raw offsets, including the skipped CR/LF code unit.
            if (ch === '\r') {
                offsets.push(i); displayed += '\n';
                if (raw.charAt(i + 1) === '\n') ++i;
            } else {
                offsets.push(i); displayed += ch;
            }
        }
        text += displayed;
        paragraphLength += displayed.length;
        spans.push({id:String(segment.id || ''), index:index, start:start,
            end:text.length, sourceStart:first, sourceEnd:last, offsets:offsets,
            startMs:Number(segment.start_ms || 0), endMs:segment.end_ms,
            speaker:speaker});
        lastSpeaker = speaker;
    });
    return {text:text, spans:spans};
}
function spanAt(document, position) {
    var spans = document.spans || [], lo = 0, hi = spans.length - 1;
    while (lo <= hi) {
        var mid = (lo + hi) >> 1, span = spans[mid];
        if (position < span.start) hi = mid - 1;
        else if (position >= span.end) lo = mid + 1;
        else return span;
    }
    // A caret at the end still belongs to the preceding passage. Synthetic
    // separators otherwise have no owner, so do not silently target a segment.
    return position === document.text.length && spans.length ? spans[spans.length-1] : null;
}
function target(document, start, end) {
    var low = Math.min(start, end), high = Math.max(start, end);
    var a = spanAt(document, low), b = spanAt(document, high > low ? high - 1 : high);
    if (!a || !b || a.index !== b.index || !a.id) return null;
    return {id:a.id, index:a.index, startMs:a.startMs, endMs:a.endMs,
        sourceStart:low === a.end ? a.sourceEnd : a.offsets[low - a.start],
        sourceEnd: high > low ? ((high === a.end) ? a.sourceEnd : a.offsets[high-a.start]) :
                               (low === a.end ? a.sourceEnd : a.offsets[low-a.start])};
}
function atTime(document, milliseconds) {
    var spans = document.spans || [];
    for (var i = 0; i < spans.length; ++i) {
        var end = spans[i].endMs !== undefined && spans[i].endMs !== null ? spans[i].endMs :
            (i + 1 < spans.length ? spans[i+1].startMs : Infinity);
        if (milliseconds >= spans[i].startMs && milliseconds < end) return spans[i];
    }
    return null;
}

// An inline edit may touch several coarse capture intervals. Split text at
// whitespace for retained segment identities, and submit every touched segment
// in one revision. This never claims word-level timing; the owner receipt pins
// the full set of original intervals.
function edit(document, changed) {
    var original = document.text, prefix = 0, suffix = 0;
    if (changed === original) return null;
    if (!changed.trim()) return {invalid:true};
    while (prefix < original.length && prefix < changed.length && original[prefix] === changed[prefix]) ++prefix;
    while (suffix < original.length - prefix && suffix < changed.length - prefix &&
           original[original.length - 1 - suffix] === changed[changed.length - 1 - suffix]) ++suffix;
    var end = original.length - suffix, spans = document.spans || [];
    var first = spans.findIndex(function(span) { return prefix <= span.end; });
    var last = first;
    while (last + 1 < spans.length && spans[last + 1].start < end) ++last;
    if (first < 0 || !spans[first].id) return {invalid:true};
    var from = spans[first].start, to = spans[last].end;
    var delta = changed.length - original.length;
    var replacement = changed.slice(from, to + delta), cuts = [0];
    for (var i = first + 1; i <= last; ++i) {
        var boundary = spans[i].start, mapped;
        if (boundary <= prefix) mapped = boundary;
        else if (boundary >= end) mapped = boundary + delta;
        else mapped = prefix + Math.round((boundary-prefix) / Math.max(1,end-prefix) * (end-prefix+delta));
        var wanted = Math.max(cuts[cuts.length-1],Math.min(replacement.length,mapped-from));
        var left = wanted, right = wanted;
        while (left > cuts[cuts.length-1] && !/\s/.test(replacement.charAt(left))) --left;
        while (right < replacement.length && !/\s/.test(replacement.charAt(right))) ++right;
        cuts.push(wanted-left <= right-wanted ? left : right);
    }
    cuts.push(replacement.length);
    var edits = [];
    for (var j = first; j <= last; ++j) {
        if (!spans[j].id) return {invalid:true};
        edits.push({segment_id:spans[j].id,text:replacement.slice(cuts[j-first],cuts[j-first+1]).trim()});
    }
    return {id:spans[first].id,index:spans[first].index,text:edits[0].text,edits:edits};
}

function applyDraft(document, segmentId, replacement) {
    var span = (document.spans || []).find(function(item) { return item.id === segmentId; });
    return span ? document.text.slice(0, span.start) + replacement + document.text.slice(span.end) : document.text;
}
