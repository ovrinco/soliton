.pragma library

// Presentation-only contracts. No owner calls, wall clock, or persisted authority.
function overflows(size, policy, alwaysOff) {
    return policy !== alwaysOff && isFinite(size) && size > 0 && size < 1 - 0.00001;
}
function previewLine(value) {
    return String(value === undefined || value === null ? '' : value).replace(/\s+/g, ' ').trim();
}
function clamp(value, low, high) { return Math.max(low, Math.min(value, Math.max(low, high))); }
function popupPosition(trigger, menu, viewport, margin) {
    var m = margin === undefined ? 8 : margin;
    var below = trigger.y + trigger.height + 4;
    var y = below + menu.height <= viewport.height - m ? below : trigger.y - menu.height - 4;
    return {x: clamp(trigger.x + trigger.width - menu.width, m, viewport.width-menu.width-m),
            y: clamp(y, m, viewport.height-menu.height-m)};
}
function literalMatches(text, query) {
    if (!query) return [];
    var escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    var re = new RegExp(escaped, 'gi'), match, matches = [];
    while ((match = re.exec(text)) !== null) matches.push({start:match.index, length:match[0].length});
    return matches;
}
function replaceMatches(text, matches, replacement) {
    var result = text;
    // Work backwards against the original offsets; replacement is literal,
    // including $&, $1, emoji, combining marks and line endings.
    for (var i = matches.length - 1; i >= 0; --i) {
        var m = matches[i];
        result = result.slice(0,m.start) + replacement + result.slice(m.start+m.length);
    }
    return result;
}

function wheelPosition(current, origin, content, viewport, pixelDelta, angleDelta, lines, lineHeight) {
    // Qt has already applied the user's natural-scroll preference to deltas.
    var delta = pixelDelta !== 0 ? pixelDelta : angleDelta / 120 * lines * lineHeight;
    return clamp(current - delta, origin, origin + Math.max(0,content-viewport));
}
