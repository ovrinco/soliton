.pragma library

// The engine's paragraphs as one readable document. Each paragraph span maps a
// UTF-16 range of the paragraph to a range of its segment. Neighbouring spans
// of one segment inside a paragraph read as one passage; a segment the engine
// split across paragraphs becomes one passage per paragraph (its parts).
// Offsets match TranscriptDocument.build, so selection, seeking and editing
// keep working. Without paragraphs the segment document is used unchanged.
function paragraphDocument(segments, paragraphs, fallback) {
    var byId = {};
    (segments || []).forEach(function(segment, index) { byId[String(segment.id || '')] = {segment: segment, index: index}; });
    if (!paragraphs || !paragraphs.length) return fallback;
    var text = '', spans = [], sources = {};
    for (var p = 0; p < paragraphs.length; ++p) {
        var paragraph = paragraphs[p], shown = String(paragraph.text || ''), base;
        if (!shown.trim()) continue;
        if (text.length) text += '\n\n';
        base = text.length;
        text += shown;
        var parts = paragraph.spans || [];
        for (var i = 0; i < parts.length; ++i) {
            var first = parts[i], id = String(first.segment_id || ''), known = byId[id];
            if (!known) return fallback;
            var raw = String(known.segment.text || ''), last = first;
            // Merge the following spans of the same segment while the gap shown
            // equals the gap in the source.
            while (i + 1 < parts.length && String(parts[i + 1].segment_id || '') === id &&
                   shown.slice(last.end, parts[i + 1].start) === raw.slice(last.source_end, parts[i + 1].source_start)) {
                last = parts[++i];
            }
            if (shown.slice(first.start, last.end) !== raw.slice(first.source_start, last.source_end)) return fallback;
            var offsets = [];
            for (var k = first.source_start; k < last.source_end; ++k) offsets.push(k);
            sources[id] = raw;
            spans.push({id: id, index: known.index, start: base + first.start, end: base + last.end,
                sourceStart: first.source_start, sourceEnd: last.source_end, offsets: offsets,
                startMs: Number(known.segment.start_ms || 0), endMs: known.segment.end_ms,
                speaker: String(known.segment.speaker || '')});
        }
    }
    return {text: text, spans: spans, sources: sources, paragraphs: paragraphs.length};
}

// An edit of the paragraph document as whole-segment texts. The touched region
// is cut between touched passages at whitespace (as TranscriptDocument.edit
// does); a segment shown in several parts keeps its untouched parts, joined
// with their own source gaps. Every edit names a whole segment once.
function paragraphEdit(document, changed) {
    if (!document.sources) return null;
    var original = document.text, prefix = 0, suffix = 0, spans = document.spans || [];
    if (changed === original) return null;
    if (!changed.trim()) return {invalid: true};
    while (prefix < original.length && prefix < changed.length && original[prefix] === changed[prefix]) ++prefix;
    while (suffix < original.length - prefix && suffix < changed.length - prefix &&
           original[original.length - 1 - suffix] === changed[changed.length - 1 - suffix]) ++suffix;
    var end = original.length - suffix, delta = changed.length - original.length;
    var first = spans.findIndex(function(span) { return prefix <= span.end; });
    if (first < 0) first = spans.length - 1;
    var last = first;
    while (last + 1 < spans.length && spans[last + 1].start < end) ++last;
    if (first < 0 || !spans[first].id) return {invalid: true};
    var from = Math.min(prefix, spans[first].start), to = Math.max(end, spans[last].end);
    var replacement = changed.slice(from, to + delta), cuts = [0];
    for (var i = first + 1; i <= last; ++i) {
        var boundary = spans[i].start, mapped;
        if (boundary <= prefix) mapped = boundary;
        else if (boundary >= end) mapped = boundary + delta;
        else mapped = prefix + Math.round((boundary - prefix) / Math.max(1, end - prefix) * (end - prefix + delta));
        var wanted = Math.max(cuts[cuts.length - 1], Math.min(replacement.length, mapped - from));
        var left = wanted, right = wanted;
        while (left > cuts[cuts.length - 1] && !/\s/.test(replacement.charAt(left))) --left;
        while (right < replacement.length && !/\s/.test(replacement.charAt(right))) ++right;
        cuts.push(wanted - left <= right - wanted ? left : right);
    }
    cuts.push(replacement.length);
    var changedParts = {}, order = [];
    for (var j = first; j <= last; ++j) {
        if (!spans[j].id) return {invalid: true};
        changedParts[j] = replacement.slice(cuts[j - first], cuts[j - first + 1]).trim();
        if (order.indexOf(spans[j].id) < 0) order.push(spans[j].id);
    }
    var edits = order.map(function(id) {
        var raw = document.sources[id], pieces = [], previous = null;
        spans.forEach(function(span, index) {
            if (span.id !== id) return;
            var piece = changedParts[index] !== undefined ? changedParts[index] : raw.slice(span.sourceStart, span.sourceEnd);
            if (piece) pieces.push((pieces.length && previous ? (raw.slice(previous.sourceEnd, span.sourceStart) || ' ') : '') + piece);
            previous = span;
        });
        return {segment_id: id, text: pieces.join('').trim()};
    });
    return {id: spans[first].id, index: spans[first].index, text: edits[0].text, edits: edits};
}

// Places the engine's word timings on the displayed transcript. Words are
// paired with the passage's own whitespace-separated words in order; a segment
// shown in several parts takes its words in order across them. A passage
// whose word count differs from the engine's carries no word marks.
function place(document, timings) {
    var bySegment = {}, used = {};
    ((timings && timings.segments) || []).forEach(function(segment) {
        bySegment[String(segment.id || '')] = segment.words || [];
    });
    var text = document.text || '', anyEstimated = false;
    var spans = (document.spans || []).map(function(span) {
        var all = bySegment[span.id] || [], found = [], pattern = /\S+/g, match;
        var shown = text.slice(span.start, span.end);
        while ((match = pattern.exec(shown)) !== null)
            found.push({start: span.start + match.index, end: span.start + match.index + match[0].length});
        var from = used[span.id] || 0;
        used[span.id] = from + found.length;
        var words = document.sources ? all.slice(from, from + found.length) : all;
        if (found.length !== words.length) return [];
        return found.map(function(place, i) {
            var estimated = words[i].estimated !== false;
            if (estimated) anyEstimated = true;
            return {start: place.start, end: place.end, startMs: Number(words[i].start_ms),
                endMs: Number(words[i].end_ms), estimated: estimated};
        });
    });
    // A segment whose parts together hold a different word count carries none.
    (document.spans || []).forEach(function(span, index) {
        if (used[span.id] !== (bySegment[span.id] || []).length) spans[index] = [];
    });
    return {spans: spans, anyEstimated: anyEstimated};
}

function wordAt(words, milliseconds) {
    var lo = 0, hi = (words || []).length - 1, best = null;
    while (lo <= hi) {
        var mid = (lo + hi) >> 1;
        if (words[mid].startMs <= milliseconds) { best = words[mid]; lo = mid + 1; }
        else hi = mid - 1;
    }
    return best && milliseconds < best.endMs ? best : null;
}

function wordAtOffset(words, offset) {
    for (var i = 0; i < (words || []).length; ++i)
        if (offset >= words[i].start && offset < words[i].end) return words[i];
    return null;
}

var speedRing = [1, 1.25, 1.5, 1.75, 2, 0.75];

function nextSpeed(current, ceiling) {
    var ring = speedRing.filter(function(rate) { return rate <= ceiling; });
    var at = ring.indexOf(current);
    if (at >= 0) return ring[(at + 1) % ring.length];
    var above = ring.filter(function(rate) { return rate > current; }).sort(function(a, b) { return a - b; });
    return above.length ? above[0] : ring[0];
}

function stepSpeed(current, direction, ceiling) {
    var stepped = Math.round((current + 0.25 * direction) * 4) / 4;
    return Math.max(0.5, Math.min(ceiling, stepped));
}

function speedLabel(rate) {
    return String(Math.round(rate * 100) / 100) + '×';
}
