pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The keyboard map in real proportions: every key is drawn whether or not
// any binding loaded; a unit key is square.
Item {
    id: root
    objectName: 'keyboard-map'
    property var bindings: []
    property string selectedKey: ''
    property string hoveredKey: ''
    // A clicked key keeps its bindings open beside it; Soliton's own are
    // edited there, in place.
    property string pinnedKey: ''
    property int editingIndex: -1
    property string pending: ''
    property string conflict: ''
    signal selected(string key)
    signal editRequested(int index)
    readonly property var physicalRows: [
        ['ESCAPE','F1','F2','F3','F4','F5','F6','F7','F8','F9','F10','F11','F12','PRINT','DELETE'],
        ['GRAVE','1','2','3','4','5','6','7','8','9','0','MINUS','EQUAL','BACKSPACE','HOME'],
        ['TAB','Q','W','E','R','T','Y','U','I','O','P','BRACKETLEFT','BRACKETRIGHT','BACKSLASH','END'],
        ['CAPS_LOCK','A','S','D','F','G','H','J','K','L','SEMICOLON','APOSTROPHE','RETURN','PAGE_UP'],
        ['SHIFT_L','Z','X','C','V','B','N','M','COMMA','PERIOD','SLASH','SHIFT_R','UP','PAGE_DOWN'],
        ['CONTROL_L','SUPER_L','ALT_L','SPACE','ALT_R','SUPER_R','CONTROL_R','LEFT','DOWN','RIGHT']
    ]
    readonly property int keyCount: physicalRows.reduce((total, row) => total + row.length, 0)
    readonly property real gap: 4
    function weight(key) {
        // US key widths in key units (a unit key is square).
        return ({SPACE:6.25, BACKSPACE:2, TAB:1.5, BACKSLASH:1.5, CAPS_LOCK:1.75, RETURN:2.25,
                 SHIFT_L:2.25, SHIFT_R:1.75, CONTROL_L:1.25, SUPER_L:1.25, ALT_L:1.25})[key] || 1;
    }
    readonly property real rowUnits: Math.max(...physicalRows.map(row => row.reduce((total, key) => total + weight(key), 0)))
    // Every row spans the same width: the widest key of a short row takes the
    // remainder, and a row of single keys shares it evenly.
    function widthOf(row, key) {
        var total = row.reduce((sum, name) => sum + weight(name), 0), spare = rowUnits - total;
        var widest = Math.max(...row.map(name => weight(name)));
        var units = widest > 1 ? weight(key) + (weight(key) === widest && row.find(name => weight(name) === widest) === key ? spare : 0)
                               : weight(key) * rowUnits / total;
        return units * unit + (units - 1) * gap;
    }
    readonly property real unit: Math.floor(Math.min((height - 5 * gap) / 6, (width - (rowUnits - 1) * gap) / rowUnits))
    function label(key) {
        return ({PRINT:'PrtSc',DELETE:'Del',HOME:'Home',END:'End',LEFT:'←',UP:'↑',RIGHT:'→',DOWN:'↓',ESCAPE:'Esc',BACKSPACE:'Backspace',RETURN:'Enter',CAPS_LOCK:'Caps',SHIFT_L:'Shift',SHIFT_R:'Shift',CONTROL_L:'Ctrl',CONTROL_R:'Ctrl',SUPER_L:'Super',SUPER_R:'Super',ALT_L:'Alt',ALT_R:'Alt',GRAVE:'`',MINUS:'−',EQUAL:'=',BRACKETLEFT:'[',BRACKETRIGHT:']',BACKSLASH:'\\',SEMICOLON:';',APOSTROPHE:"'",COMMA:',',PERIOD:'.',SLASH:'/',SPACE:'Space',PAGE_UP:'PgUp',PAGE_DOWN:'PgDn',TAB:'Tab'})[key] || key;
    }
    // Keycodes of the main block, for bindings made by keycode.
    readonly property var keycodes: ({9:'ESCAPE',10:'1',11:'2',12:'3',13:'4',14:'5',15:'6',16:'7',17:'8',18:'9',19:'0',20:'MINUS',21:'EQUAL',
        22:'BACKSPACE',23:'TAB',24:'Q',25:'W',26:'E',27:'R',28:'T',29:'Y',30:'U',31:'I',32:'O',33:'P',34:'BRACKETLEFT',35:'BRACKETRIGHT',
        36:'RETURN',38:'A',39:'S',40:'D',41:'F',42:'G',43:'H',44:'J',45:'K',46:'L',47:'SEMICOLON',48:'APOSTROPHE',49:'GRAVE',51:'BACKSLASH',
        52:'Z',53:'X',54:'C',55:'V',56:'B',57:'N',58:'M',59:'COMMA',60:'PERIOD',61:'SLASH',65:'SPACE'})
    // The physical key a chord ends on.
    function physical(chord) {
        var text = String(chord).replace(/\s+/g, '');
        var key = /\+\+$/.test(text) || text === '+' ? '+' : text.split('+').pop();
        var upper = key.toUpperCase();
        var code = /^CODE:(\d+)$/.exec(upper);
        if (code) return keycodes[code[1]] || upper;
        return ({'=':'EQUAL','+':'EQUAL','-':'MINUS','`':'GRAVE',',':'COMMA','.':'PERIOD','/':'SLASH',';':'SEMICOLON',"'":'APOSTROPHE',
                 '[':'BRACKETLEFT',']':'BRACKETRIGHT','\\':'BACKSLASH',ENTER:'RETURN',ESC:'ESCAPE',DEL:'DELETE',PGUP:'PAGE_UP',PGDOWN:'PAGE_DOWN',
                 PAGEUP:'PAGE_UP',PAGEDOWN:'PAGE_DOWN',CAPSLOCK:'CAPS_LOCK',PRINTSCREEN:'PRINT'})[upper] || upper;
    }
    function matches(key) { return bindings.filter(row => (row.keys || [row.chord]).some(chord => physical(chord) === key)); }
    // Super is implied: a key shows the core of what Super and that key do,
    // in a word or two letters short of it.
    readonly property var coreWords: ({terminal:'Term', transparency:'Transp', notification:'Notif', scratchpad:'Scratch',
        workspace:'WS', keybindings:'Keys', floating:'Float', grouping:'Group', scaling:'Scale', fullscreen:'Full'})
    readonly property var fillerWords: ['toggle','the','a','an','on','to','of','in','for','window','universal','switch','soliton',
        'omarchy','open','launch','show','quick','last','active','manager','center','monitor','focus']
    function core(action) {
        var words = String(action || '').toLowerCase().replace(/\(.*?\)/g, ' ').split(/[\s\/]+/).filter(word => word);
        var kept = words.filter(word => fillerWords.indexOf(word) < 0);
        var number = kept.find(part => /^\d+$/.test(part));
        // A workspace without its number is not the core word.
        var word = (kept[0] === 'workspace' && !number ? kept[1] : kept[0]) || words[0] || '';
        if (!word) return '';
        var short = coreWords[word] || (word.length > 7 ? word.slice(0, 5) : word.charAt(0).toUpperCase() + word.slice(1));
        return number && number !== word ? short + ' ' + number : short;
    }
    function superCore(key) {
        var row = bindings.find(row => (row.keys || [row.chord]).some(chord => {
            var parts = String(chord).replace(/\s+/g, '').split('+');
            return parts.length === 2 && parts[0] === 'Super' && physical(chord) === key;
        }));
        return row ? core(row.action) : '';
    }
    Column {
        id: keyboard
        anchors.horizontalCenter: parent.horizontalCenter
        anchors.verticalCenter: parent.verticalCenter
        spacing: root.gap
        Repeater {
            model: root.physicalRows
            delegate: Row {
                id: keyRow
                required property var modelData
                spacing: root.gap
                Repeater {
                    model: keyRow.modelData
                    delegate: Rectangle {
                        id: keyCap
                        required property string modelData
                        readonly property var bound: root.matches(keyCap.modelData)
                        readonly property bool lit: keyArea.containsMouse || root.selectedKey === keyCap.modelData || root.pinnedKey === keyCap.modelData
                        // Super is held on this map; Space and Enter are drawn as their own shapes.
                        readonly property bool held: keyCap.modelData === 'SUPER_L' || keyCap.modelData === 'SUPER_R'
                        readonly property bool special: keyCap.modelData === 'SPACE' || keyCap.modelData === 'RETURN'
                        readonly property string coreText: root.superCore(keyCap.modelData)
                        objectName: 'key-' + keyCap.modelData
                        width: root.widthOf(keyRow.modelData, keyCap.modelData); height: root.unit
                        radius: Theme.cornerRadius
                        color: lit ? Theme.selection : held ? Theme.hover : bound.length ? Theme.raised : Theme.panel
                        border.color: lit || held ? Theme.accent : bound.length ? Theme.edge : Theme.line
                        LabelText {
                            id: keyName
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.verticalCenter: parent.verticalCenter
                            anchors.verticalCenterOffset: keyCap.coreText ? -Math.round(root.unit * 0.13) : 0
                            text: keyCap.modelData === 'SPACE' ? '␣' : keyCap.modelData === 'RETURN' ? '⏎  Enter' : root.label(keyCap.modelData)
                            color: keyCap.held ? Theme.accentText : keyCap.bound.length ? Theme.text : Theme.muted
                            font.pixelSize: (keyCap.modelData === 'SPACE' ? 6 : 0) + (keyCap.bound.length || keyCap.held ? Theme.fontSizeSecondary + 1 : Theme.fontSizeSecondary)
                            font.weight: keyCap.bound.length || keyCap.held ? Font.DemiBold : Font.Normal
                        }
                        // The command core of Super and this key.
                        LabelText {
                            objectName: 'key-core-' + keyCap.modelData
                            visible: !!keyCap.coreText
                            anchors.horizontalCenter: parent.horizontalCenter
                            anchors.top: keyName.bottom; anchors.topMargin: 1
                            width: parent.width - 8; horizontalAlignment: Text.AlignHCenter; elide: Text.ElideRight
                            text: keyCap.coreText
                            color: Theme.accentText
                            font.pixelSize: Theme.fontSizeSmall - 1
                        }
                        // Space carries the bar of a space bar; Enter its return hook.
                        Rectangle {
                            visible: keyCap.special
                            anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
                            anchors.margins: 3; height: 2; radius: 1
                            color: Theme.edge
                        }
                        // A bound key carries the accent bar; more than one
                        // binding shows its count in the corner.
                        Rectangle {
                            objectName: 'key-mark-' + keyCap.modelData
                            visible: keyCap.bound.length > 0
                            anchors.bottom: parent.bottom; anchors.bottomMargin: Math.max(4, Math.round(root.unit * 0.09))
                            anchors.horizontalCenter: parent.horizontalCenter
                            width: Math.min(parent.width - 16, Math.round(root.unit * 0.42)); height: 2; radius: 1
                            color: keyCap.bound.some(row => row.editable) ? Theme.accent : Theme.muted
                        }
                        LabelText {
                            objectName: 'key-count-' + keyCap.modelData
                            visible: keyCap.bound.length > 1
                            anchors.top: parent.top; anchors.right: parent.right; anchors.margins: 5
                            text: keyCap.bound.length
                            color: Theme.muted
                            font.pixelSize: Theme.fontSizeSmall - 2
                        }
                        MouseArea {
                            id: keyArea
                            anchors.fill: parent; hoverEnabled: true
                            onContainsMouseChanged: root.hoveredKey = containsMouse ? keyCap.modelData : (root.hoveredKey === keyCap.modelData ? '' : root.hoveredKey)
                            onClicked: root.selected(keyCap.modelData)
                        }
                        Accessible.role: Accessible.Button
                        Accessible.name: root.label(keyCap.modelData)
                    }
                }
            }
        }
    }
    // The bindings of the key under the pointer, or of the clicked key,
    // beside that key. A Soliton binding is edited right here.
    Rectangle {
        id: readout
        objectName: 'key-readout'
        readonly property string key: root.pinnedKey || root.hoveredKey
        readonly property bool pinned: !!root.pinnedKey
        readonly property Item target: key ? findKey(key) : null
        readonly property var entries: key ? root.matches(key).map(row => ({row: row, index: root.bindings.indexOf(row)})) : []
        function findKey(name) {
            for (var r = 0; r < keyboard.children.length; r++) {
                var row = keyboard.children[r];
                for (var k = 0; row.children && k < row.children.length; k++)
                    if (row.children[k].objectName === 'key-' + name) return row.children[k];
            }
            return null;
        }
        visible: !!target && entries.length > 0
        z: 2
        readonly property point at: target ? target.mapToItem(root, target.width + 6, 0) : Qt.point(0, 0)
        readonly property point before: target ? target.mapToItem(root, -6 - width, 0) : Qt.point(0, 0)
        x: at.x + width <= root.width ? at.x : Math.max(0, before.x)
        y: Math.max(0, Math.min(at.y, root.height - height))
        width: entryColumn.width + 12; height: entryColumn.height + 12
        color: Theme.raised; border.color: pinned ? Theme.accent : Theme.line; radius: Theme.cornerRadius
        Column {
            id: entryColumn
            x: 6; y: 6
            // The chord column is as wide as the longest chord (mono type).
            readonly property real chordWidth: chordMetrics.advanceWidth * Math.max(18, ...readout.entries.map(entry => String(entry.row.chord).length), root.pending.length)
            TextMetrics { id: chordMetrics; font.family: Theme.mono; font.pixelSize: Theme.fontSizeSecondary; text: 'M' }
            Repeater {
                model: readout.entries
                delegate: Rectangle {
                    id: entry
                    required property var modelData
                    readonly property bool editable: entry.modelData.row.editable && readout.pinned
                    readonly property bool editing: root.editingIndex >= 0 && root.editingIndex === entry.modelData.index
                    objectName: 'readout-entry-' + entry.modelData.index
                    width: entryRow.implicitWidth + 12; height: Math.max(26, entryRow.implicitHeight + 8)
                    radius: Theme.cornerRadius
                    color: editing ? Theme.base : entryArea.containsMouse && editable ? Theme.hover : 'transparent'
                    border.color: editing ? Theme.accent : 'transparent'
                    Row {
                        id: entryRow
                        x: 6; anchors.verticalCenter: parent.verticalCenter
                        spacing: 12
                        LabelText {
                            width: entryColumn.chordWidth
                            font.family: Theme.mono; font.pixelSize: Theme.fontSizeSecondary
                            text: entry.editing ? (root.pending || 'Press the new keys') : entry.modelData.row.chord
                            color: entry.editing && !root.pending ? Theme.muted : Theme.text
                        }
                        LabelText {
                            font.pixelSize: Theme.fontSizeSecondary
                            text: entry.modelData.row.action
                            color: entry.modelData.row.editable ? Theme.text : Theme.muted
                        }
                        LabelText {
                            objectName: 'readout-conflict'
                            visible: entry.editing && !!root.conflict
                            font.pixelSize: Theme.fontSizeSecondary
                            text: 'Already used by ' + root.conflict
                            color: Theme.dangerText
                        }
                    }
                    MouseArea {
                        id: entryArea
                        anchors.fill: parent; hoverEnabled: true
                        enabled: entry.editable
                        cursorShape: enabled ? Qt.PointingHandCursor : Qt.ArrowCursor
                        onClicked: root.editRequested(entry.modelData.index)
                    }
                }
            }
        }
    }
}
