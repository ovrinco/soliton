import QtQuick
import QtQuick.Layouts

// A record's tracker row in one line: its status word as the store spells it, in its tone, and its purpose.
// Read once per row and again whenever the store changes while shown; nothing shows when the row is absent.
RowLayout {
    id: link
    objectName: 'row-link'
    property string rowId: ''
    property var row: null
    readonly property string word: row ? String(row.status || '') : ''
    readonly property string tone: ['verified', 'implemented'].indexOf(word) >= 0 ? 'success'
        : ['in build', 'Planning', 'planned'].indexOf(word) >= 0 ? 'accent'
        : ['built wrong', 'rejected'].indexOf(word) >= 0 ? 'danger' : 'neutral'
    property bool watching: false
    visible: !!row
    spacing: Theme.space8

    function read() {
        var id = link.rowId;
        if (!id) { link.row = null; return; }
        Backend.send('tracker-row', {id: id}, (r, ok) => {
            if (id !== link.rowId) return;
            link.row = ok && r && r.row ? r.row : null;
        }, true);
    }
    function watch(on) {
        if (on === link.watching) return;
        link.watching = on;
        Backend.watchTracker(on);
    }
    onRowIdChanged: { link.watch(!!rowId); link.read(); }
    Component.onCompleted: { link.watch(!!rowId); link.read(); }
    Component.onDestruction: link.watch(false)
    Connections {
        target: Backend
        function onTrackerRevisionChanged() { if (link.rowId) link.read(); }
    }

    LabelText {
        objectName: 'row-link-status'
        text: link.word
        font.pixelSize: Theme.fontSizeSmall
        font.weight: Font.Medium
        color: link.tone === 'accent' ? Theme.accentText
            : link.tone === 'success' ? Theme.successText
            : link.tone === 'danger' ? Theme.dangerText
            : Theme.muted
    }
    LabelText {
        objectName: 'row-link-purpose'
        Layout.fillWidth: true
        text: link.row ? String(link.row.purpose || '') : ''
        elide: Text.ElideRight
        maximumLineCount: 1
        font.pixelSize: Theme.fontSizeSmall
        color: Theme.muted
    }
}
