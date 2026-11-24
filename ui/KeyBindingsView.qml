pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Settings › Key bindings: the bindings above, the keyboard map below, two
// halves of equal height. Soliton's chords are edited in place, in the list
// or on the map beside the clicked key.
Item {
    id: root
    objectName: 'key-bindings-view'
    property bool active: false
    property var desktopRows: []
    property string error: ''
    property bool loaded: false
    property int selectedIndex: -1
    property int editingIndex: -1
    property string pending: ''
    // Soliton's own rows never depend on reading the desktop's.
    readonly property var rows: SolitonKeys.rows.concat(desktopRows)
    readonly property var selectedRow: selectedIndex >= 0 && selectedIndex < rows.length ? rows[selectedIndex] : null
    readonly property string conflict: {
        if (editingIndex < 0 || !pending) return '';
        var wanted = SolitonKeys.normalize(pending);
        var other = rows.find((row, index) => index !== editingIndex && row.keys.some(chord => SolitonKeys.normalize(chord) === wanted));
        return other ? other.action : '';
    }
    function load() {
        SystemSettings.request({action: 'bindings'}, (result, failure) => {
            error = failure || ''; loaded = !failure;
            desktopRows = failure ? [] : (result.items || []).map(row => {
                var chord = String(row.combo || '').split(' + ').map(part => ({SUPER: 'Super', CTRL: 'Ctrl', ALT: 'Alt', SHIFT: 'Shift'})[part]
                    || (part.length > 1 && part === part.toUpperCase() && !/^code:/.test(part) ? part.charAt(0) + part.slice(1).toLowerCase() : part)).join('+');
                var action = row.description || [row.dispatcher, row.arg].filter(part => part).join(' ');
                return {id: '', action: action, chord: chord, scope: 'Desktop', editable: false, keys: [chord]};
            });
        });
    }
    onActiveChanged: if (active) load()
    Connections {
        target: SystemSettings
        function onChanged(domain) { if (root.active && domain === 'devices' && root.editingIndex < 0) root.load(); }
    }
    function select(index) {
        if (editingIndex >= 0 && index !== editingIndex) cancel();
        if (index >= 0 && rows[index] && map.physical(rows[index].keys[0]) !== map.pinnedKey) map.pinnedKey = '';
        selectedIndex = index;
        if (index >= 0) list.positionViewAtIndex(index, ListView.Contain);
    }
    function edit(index) {
        if (!rows[index] || !rows[index].editable) return;
        selectedIndex = index; editingIndex = index; pending = '';
        capture.forceActiveFocus();
    }
    function cancel() { editingIndex = -1; pending = ''; map.pinnedKey = ''; }
    // A click on the map opens the key's bindings beside it; a key with one
    // Soliton binding goes straight to editing it. A second click closes.
    function pin(key) {
        if (key === map.pinnedKey) { cancel(); return; }
        var indices = [];
        rows.forEach((row, index) => { if (row.keys.some(chord => map.physical(chord) === key)) indices.push(index); });
        cancel();
        if (!indices.length) { selectedIndex = -1; return; }
        selectedIndex = indices[0];
        list.positionViewAtIndex(indices[0], ListView.Contain);
        map.pinnedKey = key;
        var editable = indices.filter(index => rows[index].editable);
        if (editable.length === 1) edit(editable[0]);
    }
    function apply() {
        if (editingIndex < 0 || !pending || conflict) return;
        SolitonKeys.assign(rows[editingIndex].id, pending);
        cancel();
    }
    // The chord a key press spells; empty for a lone modifier or a bare key.
    function chordOf(event) {
        var mods = [];
        if (event.modifiers & Qt.ControlModifier) mods.push('Ctrl');
        if (event.modifiers & Qt.AltModifier) mods.push('Alt');
        if (event.modifiers & Qt.ShiftModifier) mods.push('Shift');
        if (event.modifiers & Qt.MetaModifier) mods.push('Meta');
        var key = '', code = event.key;
        if (code >= Qt.Key_F1 && code <= Qt.Key_F24) key = 'F' + (code - Qt.Key_F1 + 1);
        else if (code > 0x20 && code < 0x7f) {
            key = String.fromCharCode(code).toUpperCase();
            key = ({'!':'1','@':'2','#':'3','$':'4','%':'5','^':'6','&':'7','*':'8','(':'9',')':'0','_':'-','+':'=',
                    '?':'/',':':';','"':"'",'<':',','>':'.','{':'[','}':']','|':'\\','~':'`'})[key] || key;
        } else key = ({[Qt.Key_Space]:'Space',[Qt.Key_Tab]:'Tab',[Qt.Key_Backspace]:'Backspace',[Qt.Key_Delete]:'Del',
                       [Qt.Key_Home]:'Home',[Qt.Key_End]:'End',[Qt.Key_PageUp]:'PgUp',[Qt.Key_PageDown]:'PgDown',
                       [Qt.Key_Left]:'Left',[Qt.Key_Right]:'Right',[Qt.Key_Up]:'Up',[Qt.Key_Down]:'Down',
                       [Qt.Key_Return]:'Return',[Qt.Key_Enter]:'Return',[Qt.Key_Escape]:'Esc'})[code] || '';
        if (!key) return '';
        var function_key = /^F\d+$/.test(key);
        if (!function_key && !mods.some(mod => mod !== 'Shift')) return '';
        return mods.concat([key]).join('+');
    }
    Item {
        id: capture
        objectName: 'chord-capture'
        Keys.onShortcutOverride: event => event.accepted = root.editingIndex >= 0
        Keys.onPressed: event => {
            if (root.editingIndex < 0) return;
            event.accepted = true;
            var bare = !(event.modifiers & (Qt.ControlModifier | Qt.AltModifier | Qt.ShiftModifier | Qt.MetaModifier));
            if (bare && (event.key === Qt.Key_Return || event.key === Qt.Key_Enter)) { root.apply(); return; }
            if (bare && event.key === Qt.Key_Escape) { root.cancel(); return; }
            var chord = root.chordOf(event);
            if (chord) root.pending = chord;
        }
        onActiveFocusChanged: if (!activeFocus && root.editingIndex >= 0) root.cancel()
    }
    Item {
        id: upper
        objectName: 'bindings-half'
        width: parent.width; height: Math.floor(parent.height / 2)
        ColumnLayout {
            anchors.fill: parent; anchors.bottomMargin: 8
            spacing: 4
            LabelText {
                objectName: 'bindings-failure'
                visible: !!root.error
                Layout.fillWidth: true; elide: Text.ElideRight
                readonly property string reason: root.error.replace(/\[Errno \d+\]\s*/g, '').replace(/:?\s*'?\/[^\s']*'?/g, '').trim()
                text: "The desktop's key bindings could not be read" + (reason ? ': ' + reason.charAt(0).toLowerCase() + reason.slice(1) : '') + '.'
                color: Theme.dangerText
            }
            RowLayout {
                Layout.fillWidth: true; Layout.leftMargin: 10; Layout.rightMargin: 10
                LabelText { text: 'Chord'; color: Theme.muted; Layout.preferredWidth: 260 }
                LabelText { text: 'Action'; color: Theme.muted; Layout.fillWidth: true }
                LabelText { text: 'Scope'; color: Theme.muted; Layout.preferredWidth: 90 }
            }
            ListView {
                id: list
                objectName: 'bindings-list'
                Layout.fillWidth: true; Layout.fillHeight: true; clip: true
                model: root.rows
                currentIndex: root.selectedIndex
                delegate: ItemDelegate {
                    id: rowItem
                    required property var modelData
                    required property int index
                    readonly property bool editing: root.editingIndex === index
                    objectName: 'binding-row-' + index
                    width: list.width; height: Theme.denseRowHeight
                    highlighted: root.selectedIndex === index
                    onClicked: root.select(index)
                    background: Rectangle { color: rowItem.highlighted ? Theme.selection : rowItem.hovered ? Theme.hover : 'transparent' }
                    contentItem: RowLayout {
                        spacing: 8
                        Rectangle {
                            objectName: 'chord-cell-' + rowItem.index
                            Layout.preferredWidth: 252; Layout.fillHeight: true
                            color: rowItem.editing ? Theme.raised : 'transparent'
                            border.color: rowItem.editing ? Theme.accent : rowItem.highlighted && rowItem.modelData.editable ? Theme.line : 'transparent'
                            radius: Theme.cornerRadius
                            LabelText {
                                anchors.verticalCenter: parent.verticalCenter
                                x: 6; width: parent.width - 12; elide: Text.ElideRight
                                font.family: Theme.mono
                                text: rowItem.editing ? (root.pending || 'Press the new keys') : rowItem.modelData.chord
                                color: rowItem.editing && !root.pending ? Theme.muted : Theme.text
                            }
                            MouseArea {
                                anchors.fill: parent
                                enabled: rowItem.modelData.editable
                                cursorShape: enabled && rowItem.highlighted ? Qt.IBeamCursor : Qt.ArrowCursor
                                onClicked: rowItem.highlighted ? root.edit(rowItem.index) : root.select(rowItem.index)
                            }
                        }
                        LabelText {
                            objectName: 'chord-conflict'
                            visible: rowItem.editing && !!root.conflict
                            text: 'Already used by ' + root.conflict
                            color: Theme.dangerText
                        }
                        LabelText { Layout.fillWidth: true; text: rowItem.modelData.action; elide: Text.ElideRight }
                        LabelText { Layout.preferredWidth: 90; text: rowItem.modelData.scope; color: Theme.muted }
                    }
                }
                ScrollBar.vertical: SScrollBar {}
            }
        }
    }
    SettingsKeyboard {
        id: map
        y: upper.height
        width: parent.width; height: parent.height - upper.height
        bindings: root.rows
        selectedKey: root.selectedRow ? physical(root.selectedRow.keys[0]) : ''
        editingIndex: root.editingIndex
        pending: root.pending
        conflict: root.conflict
        onSelected: key => root.pin(key)
        onEditRequested: index => root.edit(index)
    }
}
