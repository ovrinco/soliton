pragma Singleton
import QtQuick
import Quickshell
import Quickshell.Io

// Soliton's own key bindings. Defaults are the installed chords; a changed
// chord is stored in Soliton's data, never in the desktop's configuration.
QtObject {
    id: root
    readonly property var defaults: [
        {id: 'palette', action: 'Open the palette', chord: 'Ctrl+K'},
        {id: 'rail', action: 'Fold the side panel', chord: 'Ctrl+B'},
        {id: 'pane', action: 'Cycle the right pane', chord: 'Ctrl+J'},
        {id: 'task', action: 'New task', chord: 'Ctrl+N'},
        {id: 'note', action: 'New note', chord: 'Ctrl+Shift+N'},
        {id: 'save', action: 'Save', chord: 'Ctrl+S'},
        {id: 'focus', action: 'Focus mode', chord: 'Ctrl+Shift+F'},
        {id: 'console', action: 'Console', chord: 'Ctrl+Shift+A'},
        {id: 'private', action: 'Private mode', chord: 'Ctrl+Shift+P'}
    ]
    // The interface scale keys stay bound as installed (R-SOLITON-051).
    readonly property var scaleKeys: [
        {action: 'Larger interface', chords: ['Ctrl+=', 'Ctrl+Shift+=', 'Ctrl++']},
        {action: 'Smaller interface', chords: ['Ctrl+-', 'Ctrl+Shift+-']},
        {action: 'Interface at full size', chords: ['Ctrl+0', 'Ctrl+Shift+0']}
    ]
    property var stored: ({})
    readonly property var map: {
        var result = {};
        defaults.forEach(row => result[row.id] = stored[row.id] || row.chord);
        return result;
    }
    readonly property var rows: defaults.map(row => ({id: row.id, action: row.action, chord: map[row.id], scope: 'Soliton', editable: true, keys: [map[row.id]]}))
        .concat(scaleKeys.map(row => ({id: '', action: row.action, chord: row.chords.join('  '), scope: 'Soliton', editable: false, keys: row.chords})))
    function chord(id) { return map[id] || ''; }
    // One spelling for comparing chords from both scopes.
    function normalize(chord) {
        var parts = String(chord).replace(/\s+/g, '').split('+');
        var key = parts.pop() || '+';
        if (key === '' && parts.length) { parts.pop(); key = '+'; }
        var alias = {RETURN: 'ENTER', ESCAPE: 'ESC', EQUAL: '=', MINUS: '-', PLUS: '+', CONTROL: 'CTRL', META: 'SUPER', WIN: 'SUPER'};
        key = key.toUpperCase(); key = alias[key] || key;
        var mods = parts.map(part => { var upper = part.toUpperCase(); return alias[upper] || upper; }).filter(part => part).sort();
        return mods.concat([key]).join('+');
    }
    readonly property string storePath: Quickshell.env('SOLITON_DATA') + '/keybindings.json'
    property FileView file: FileView {
        path: root.storePath
        preload: true
        printErrors: false
        onLoaded: {
            try { var parsed = JSON.parse(text()); root.stored = parsed && parsed.chords ? parsed.chords : {}; }
            catch (error) { root.stored = {}; }
        }
    }
    // Stores a new chord for one of Soliton's actions; the caller has
    // already refused chords in use.
    function assign(id, chord) {
        if (!defaults.some(row => row.id === id)) return false;
        var next = Object.assign({}, stored);
        if (chord === defaults.find(row => row.id === id).chord) delete next[id];
        else next[id] = chord;
        stored = next;
        file.setText(JSON.stringify({chords: next}, null, 2) + '\n');
        return true;
    }
}
