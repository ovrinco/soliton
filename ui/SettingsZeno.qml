pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

// Zeno's own settings, read from and written to Zeno's profile files through
// the engine: one row per field the engine names, in Settings' own style.
Item {
    id: root
    objectName: 'zeno-settings-page'
    property bool active: false
    property var fields: []
    property var values: ({})
    property bool loaded: false
    property bool busy: false
    property string error: ''
    property string message: ''
    property var refusals: ({})
    property var saved: ({})
    readonly property string home: Quickshell.env('HOME') || ''
    readonly property string testRoot: Quickshell.env('SOLITON_ZENO_TEST_ROOT') || ''
    readonly property var rows: fields.map(row)
    readonly property int editableCount: fields.filter(f => f.editable).length
    readonly property var sectionNames: [
        {key: 'zeno', name: 'Subscription route', note: 'Used when Zeno runs on the subscription model.'},
        {key: 'zeno-local', name: 'Local route', note: 'Used when Zeno runs on the model served on this computer.'},
        {key: 'zeno-openrouter', name: 'OpenRouter route', note: 'Used when Zeno runs through OpenRouter.'},
        {key: 'catalogue', name: 'Model descriptions', note: "Kept by Zeno's release; shown here for reference."},
        {key: 'launcher', name: 'Chosen at launch', note: 'Given each time Zeno starts; Zeno keeps no file for these.'}
    ]
    readonly property var words: ({
        none: 'None', minimal: 'Minimal', low: 'Low', medium: 'Medium', high: 'High', xhigh: 'Extra high',
        untrusted: 'Ask unless the command is known safe', 'on-failure': 'Ask when a command fails',
        'on-request': 'Ask when the model requests it', never: 'Never ask',
        'read-only': 'Read only', 'workspace-write': 'Write inside the project folder', 'danger-full-access': 'Full access',
        disabled: 'Off', cached: 'Cached results', live: 'Live', responses: 'Responses',
        freeform: 'Free form', list: 'Listed', tokens: 'Tokens', shell_command: 'Shell command',
        zeno: 'Subscription', 'zeno-local': 'Local', 'zeno-openrouter': 'OpenRouter'
    })
    // Launch values the engine explains by their variable names; these say the same in plain words.
    readonly property var launchWords: ({
        home_folder: 'Folder Zeno starts in unless another folder is given at launch',
        route: 'Route Zeno uses unless another is given at launch',
        state_home: 'Where Zeno keeps its profiles and chat history',
        harness: 'The program Zeno runs on',
        agent_folder: 'Folder of the alignment files Zeno reads before its first task',
        remote_address: 'Address on which Zeno accepts remote connections',
        remote_runtime: "Folder holding Zeno's connection files while it runs",
        openrouter_key_present: 'Whether an OpenRouter key is supplied; the key itself is never read',
        remote_token_present: 'Whether a remote access token is supplied; the token itself is never read',
        native_arguments: 'Extra options given to Zeno for one launch'
    })

    function sectionOf(id) { return id.split('/')[0]; }
    function leaf(id) {
        var key = id.slice(id.indexOf('/') + 1);
        var parts = key.split(/[./]/);
        return parts[parts.length - 1];
    }
    function lastPart(path) {
        if (path === home) return 'Home folder';
        var parts = path.split('/').filter(p => p.length);
        return parts.length ? parts[parts.length - 1] : path;
    }
    // A folder relative to home, in words: Home folder › 02A_ovrin › zeno.
    function folderWords(path) {
        if (path === home) return 'Home folder';
        var inside = home && path.indexOf(home + '/') === 0 ? path.slice(home.length + 1) : path;
        return ['Home folder'].concat(inside.split('/').filter(p => p.length)).join(' › ');
    }
    // Absolute file and folder names inside text show by their last part only.
    function plain(text) {
        return String(text).replace(/(^|[\s"'(])(\/[^\s"'),]+)/g, (m, lead, path) => lead + lastPart(path.replace(/[.,;:]+$/, '')))
            .replace(/\bnull\b/g, 'empty');
    }
    // A catalogue model name without its vendor prefix: qwen/name:free reads name (free).
    function modelWords(name) {
        var bare = name.slice(name.lastIndexOf('/') + 1);
        var tag = bare.indexOf(':');
        return tag > 0 ? bare.slice(0, tag) + ' (' + bare.slice(tag + 1) + ')' : bare;
    }
    function choiceWord(value) {
        if (value === true) return 'Yes';
        if (value === false) return 'No';
        var w = words[value];
        return w !== undefined ? w : String(value);
    }
    function valueWords(f) {
        var v = f.current_value, key = leaf(f.field);
        if (v === null || v === undefined) return 'Default';
        if (typeof v === 'boolean') return v ? 'Yes' : 'No';
        if (Array.isArray(v)) return v.length ? v.map(x => typeof x === 'object' ? JSON.stringify(x) : String(x)).join(', ') : 'None';
        if (typeof v === 'object') return JSON.stringify(v);
        if (key === 'trust_level') return v === 'trusted' ? 'Trusted' : 'Not trusted';
        if (key === 'remote_address') return v.indexOf('127.0.0.1:') === 0 ? 'This computer only, port ' + v.slice(10) : v;
        if (typeof v === 'string' && v.indexOf('/') === 0) return v === home ? 'Home folder' : lastPart(v);
        if (typeof v === 'string' && words[v] !== undefined) return words[v];
        return plain(v);
    }
    function context(f) {
        var id = f.field, section = sectionOf(id);
        var project = id.match(/projects\."([^"]+)"\.trust_level$/);
        if (project) return folderWords(project[1]);
        var model = id.match(/^catalogue\/models\/(\d+)\//);
        if (model) {
            var name = fields.find(x => x.field === 'catalogue/models/' + model[1] + '/display_name');
            var level = id.match(/supported_reasoning_levels\/(\d+)\//);
            return (name ? modelWords(String(name.current_value)) : 'Model ' + (Number(model[1]) + 1)) + (level ? ' · reasoning level ' + (Number(level[1]) + 1) : '');
        }
        if (id.indexOf('.model_providers.') >= 0 || id.indexOf('/model_providers.') >= 0) return 'Provider';
        if (section === 'launcher' || section === 'catalogue') return '';
        return '';
    }
    function control(f) {
        if (!f.editable) return 'value';
        var a = f.allowed_values || [];
        if (f.kind === 'boolean') return 'switch';
        if (f.kind === 'string' && a.length > 0 && ['nonempty text'].indexOf(a[0]) < 0) return 'choice';
        return 'field';
    }
    // A row's value as the engine last read it, in words.
    function shown(r) { return valueWords(Object.assign({}, r.source, {current_value: values[r.field]})); }
    function row(f) {
        var section = sectionOf(f.field);
        var key = leaf(f.field);
        return {
            field: f.field,
            section: section,
            sectionName: (sectionNames.find(s => s.key === section) || {name: 'Other settings'}).name,
            label: section === 'launcher' && launchWords[key] ? launchWords[key] : plain(f.description),
            context: context(f),
            kind: f.kind,
            control: control(f),
            choices: f.allowed_values || [],
            labels: key === 'trust_level' ? {trusted: 'Trusted', untrusted: 'Not trusted'} : words,
            source: f,
            isPath: typeof f.current_value === 'string' && f.current_value.indexOf('/') === 0,
            note: !f.editable && section !== 'launcher' && section !== 'catalogue' ? 'Read only here' : ''
        };
    }
    // The engine's refusal, in words a person reads beside the field.
    function refusalWords(text) {
        var m = String(text || '').match(/^Value refused for .+?\. Allowed values: (\[.*\]) \((\w+) required\)$/);
        if (!m) return 'Not saved: ' + plain(text || 'Zeno did not accept this value.');
        var allowed = [];
        try { allowed = JSON.parse(m[1]); } catch (e) {}
        if (allowed[0] === 'nonempty text') return 'Not saved: this setting needs some text.';
        if (allowed[0] === 'nonnegative number') return 'Not saved: this setting needs a number of zero or more.';
        return 'Not saved: choose ' + allowed.map(choiceWord).join(', ') + '.';
    }
    function args(extra) {
        return Object.assign(testRoot ? {test_root: testRoot} : {}, extra || {});
    }
    function load(after) {
        busy = true;
        Backend.send('zeno-settings-get', args(), (result, ok, failure) => {
            busy = false; loaded = true;
            if (!ok) { error = failure || 'Zeno settings could not be read.'; return; }
            error = '';
            var next = result.fields || [];
            var map = {};
            next.forEach(f => map[f.field] = f.current_value);
            // The same fields again change only their values, so the rows and the reader's place stay.
            if (next.map(f => f.field).join('\n') !== fields.map(f => f.field).join('\n')) fields = next;
            values = map;
            if (after) after();
        }, true);
    }
    function setRefusal(id, text) {
        var r = Object.assign({}, refusals);
        if (text) r[id] = text; else delete r[id];
        refusals = r;
    }
    function setSaved(id, text) {
        var s = Object.assign({}, saved);
        if (text) s[id] = text; else delete s[id];
        saved = s;
    }
    function save(id, value) {
        if (busy) return;
        var f = fields.find(x => x.field === id);
        if (!f || !f.editable) return;
        if (f.kind === 'number' && typeof value === 'string' && /^\s*-?\d+(\.\d+)?\s*$/.test(value)) value = Number(value);
        busy = true; message = ''; setSaved(id, ''); setRefusal(id, '');
        Backend.send('zeno-settings-set', args({field: id, value: value}), (result, ok, failure) => {
            busy = false;
            if (!ok) { setRefusal(id, refusalWords(failure)); load(); return; }
            message = result && result.effects ? result.effects : '';
            // Read back from Zeno's file: the row shows what the file now holds.
            load(() => {
                var now = values[id];
                if (JSON.stringify(now) === JSON.stringify(value)) setSaved(id, 'Saved');
                else setRefusal(id, 'Saved, but Zeno now reads ' + valueWords(Object.assign({}, f, {current_value: now})) + '.');
            });
        }, true);
    }

    // Read afresh each time the page is opened; there is no refresh button.
    onActiveChanged: if (active && !busy) load()

    IpcHandler {
        enabled: Quickshell.env('SOLITON_AUTOMATION') === '1'
        target: 'soliton-zeno-settings'
        function summary(): string {
            return JSON.stringify({loaded: root.loaded, error: root.error, message: root.message, count: root.rows.length,
                editable: root.editableCount, sections: root.sectionNames.map(s => s.name)});
        }
        // A slice of the rows as shown, with each field's value as the engine read it.
        function rows(offset: int, count: int): string {
            return JSON.stringify(root.rows.slice(offset, offset + count).map(r => ({field: r.field, section: r.sectionName,
                label: r.label, context: r.context, note: r.note, value: root.shown(r), control: r.control, current: root.values[r.field],
                refusal: root.refusals[r.field] || '', saved: root.saved[r.field] || ''})));
        }
        function reveal(field: string): void {
            var i = root.rows.findIndex(r => r.field === field);
            if (i >= 0) list.positionViewAtIndex(i, ListView.Beginning);
        }
        function reload(): void { root.load(); }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 12
        RowLayout {
            Layout.fillWidth: true
            ColumnLayout {
                spacing: 4
                SectionLabel { objectName: 'zeno-settings-title'; text: 'Zeno' }
                LabelText {
                    objectName: 'zeno-settings-counts'
                    text: root.loaded && !root.error ? root.rows.length + ' settings  ·  ' + root.editableCount + ' can be changed here' : ' '
                    color: Theme.muted
                }
            }
            Item { Layout.fillWidth: true }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: root.sectionNames
                delegate: ManagerButton {
                    required property var modelData
                    objectName: 'zeno-section-' + modelData.key
                    text: modelData.name
                    selected: list.currentSection === modelData.name
                    enabled: root.rows.some(r => r.section === modelData.key)
                    onClicked: list.positionViewAtIndex(root.rows.findIndex(r => r.section === modelData.key), ListView.Beginning)
                }
            }
            Item { Layout.fillWidth: true }
        }
        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.line }
        ListView {
            id: list
            objectName: 'zeno-settings-list'
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.minimumHeight: 0
            clip: true
            spacing: 0
            boundsBehavior: Flickable.StopAtBounds
            model: root.rows
            ScrollBar.vertical: SScrollBar {}
            section.property: 'sectionName'
            section.delegate: ColumnLayout {
                id: heading
                required property string section
                width: list.width - 12
                spacing: 4
                Item { Layout.preferredHeight: 14 }
                SectionLabel { text: heading.section }
                LabelText {
                    Layout.fillWidth: true
                    text: (root.sectionNames.find(s => s.name === heading.section) || {note: ''}).note
                    color: Theme.muted
                }
                Item { Layout.preferredHeight: 6 }
            }
            delegate: Item {
                id: entry
                required property var modelData
                required property int index
                readonly property string refusal: root.refusals[modelData.field] || ''
                readonly property string savedText: root.saved[modelData.field] || ''
                readonly property var current: root.values[modelData.field]
                readonly property string shown: root.shown(modelData)
                objectName: 'zeno-row-' + index
                width: list.width - 12
                height: Math.max(labels.implicitHeight, controls.implicitHeight) + 16
                RowLayout {
                    anchors.fill: parent
                    anchors.topMargin: 8
                    anchors.bottomMargin: 8
                    spacing: 16
                    ColumnLayout {
                        id: labels
                        Layout.preferredWidth: Math.max(240, entry.width * .45)
                        Layout.minimumWidth: Math.max(240, entry.width * .45)
                        Layout.maximumWidth: Math.max(240, entry.width * .45)
                        Layout.alignment: Qt.AlignTop
                        spacing: 2
                        LabelText { Layout.fillWidth: true; text: entry.modelData.label; wrapMode: Text.WordWrap; elide: Text.ElideNone }
                        LabelText { Layout.fillWidth: true; visible: text !== ''; text: entry.modelData.context; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall }
                        LabelText { Layout.fillWidth: true; visible: text !== ''; text: entry.modelData.note; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall; wrapMode: Text.WordWrap; elide: Text.ElideNone }
                    }
                    ColumnLayout {
                        id: controls
                        Layout.fillWidth: true
                        Layout.alignment: Qt.AlignTop
                        spacing: 4
                        RowLayout {
                            Layout.fillWidth: true
                            spacing: 8
                            LabelText {
                                objectName: 'zeno-value-' + entry.index
                                visible: entry.modelData.control === 'value'
                                Layout.fillWidth: true
                                text: entry.shown
                                color: Theme.muted
                                maximumLineCount: 2
                                wrapMode: Text.WordWrap
                            }
                            SettingsSwitch {
                                objectName: 'zeno-switch-' + entry.index
                                visible: entry.modelData.control === 'switch'
                                checked: entry.current === true
                                enabled: !root.busy
                                Accessible.name: entry.modelData.label
                                onClicked: root.save(entry.modelData.field, !(entry.current === true))
                            }
                            LabelText { visible: entry.modelData.control === 'switch'; text: entry.shown; color: Theme.muted }
                            Choice {
                                objectName: 'zeno-choice-' + entry.index
                                visible: entry.modelData.control === 'choice'
                                Layout.preferredWidth: 300
                                model: entry.modelData.control === 'choice' ? entry.modelData.choices : []
                                labelMap: entry.modelData.labels
                                displayText: currentIndex >= 0 ? textFor(entry.modelData.choices[currentIndex]) : entry.shown
                                currentIndex: entry.modelData.choices.indexOf(entry.current)
                                enabled: !root.busy
                                Accessible.name: entry.modelData.label
                                onActivated: index => root.save(entry.modelData.field, entry.modelData.choices[index])
                            }
                            Field {
                                id: input
                                objectName: 'zeno-field-' + entry.index
                                visible: entry.modelData.control === 'field'
                                Layout.fillWidth: true
                                text: entry.modelData.isPath ? '' : (entry.modelData.control === 'field' ? String(entry.current) : '')
                                placeholderText: entry.modelData.isPath ? entry.shown + '  ·  type a full location to change' : ''
                                invalid: entry.refusal !== ''
                                errorText: entry.refusal
                                enabled: !root.busy
                                Accessible.name: entry.modelData.label
                                onAccepted: root.save(entry.modelData.field, input.text)
                            }
                            ManagerButton {
                                objectName: 'zeno-save-' + entry.index
                                visible: entry.modelData.control === 'field' && (entry.modelData.isPath ? input.text !== '' : input.text !== String(entry.current))
                                text: 'Save'
                                enabled: !root.busy
                                onClicked: root.save(entry.modelData.field, input.text)
                            }
                            LabelText {
                                objectName: 'zeno-saved-' + entry.index
                                visible: entry.savedText !== '' && entry.refusal === ''
                                text: entry.savedText
                                color: Theme.accentText
                            }
                            Item { Layout.fillWidth: true; visible: entry.modelData.control === 'choice' || entry.modelData.control === 'switch' }
                        }
                        LabelText {
                            objectName: 'zeno-refusal-' + entry.index
                            Layout.fillWidth: true
                            visible: entry.refusal !== ''
                            text: entry.refusal
                            color: Theme.dangerText
                            wrapMode: Text.WordWrap
                            elide: Text.ElideNone
                        }
                    }
                }
            }
            LabelText {
                anchors.centerIn: parent
                width: parent.width - 40
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                visible: root.loaded && list.count === 0
                color: root.error ? Theme.dangerText : Theme.muted
                text: root.error ? 'Zeno settings could not be read. ' + root.plain(root.error) : 'Zeno reports no settings.'
            }
        }
        LabelText {
            objectName: 'zeno-settings-status'
            Layout.fillWidth: true
            visible: text !== ''
            text: root.error && root.rows.length ? root.plain(root.error) : root.message
            color: root.error ? Theme.dangerText : Theme.muted
            font.pixelSize: Theme.fontSizeSmall
        }
    }
}
