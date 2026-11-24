pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The bar's plugin manager (ian.plugin-manager) inside Soliton's frame: its
// title with counts, search, category tabs, rows and detail pane with bar
// placement, on the manager's own engine through the settings bridge.
Item {
    id: root
    objectName: 'plugins-page'
    property bool active: false
    property var rows: []
    property bool loaded: false
    property bool busy: false
    property string error: ''
    property string message: ''
    property string query: ''
    property string category: 'all'
    property string selectedId: ''
    readonly property var installed: rows.filter(p => p.installed)
    readonly property int enabledCount: installed.filter(p => p.enabled).length
    readonly property var selected: rows.find(p => p.id === selectedId) || null
    readonly property var categories: [{key: 'all', name: 'All installed'}, {key: 'user', name: 'Added by you'}, {key: 'bar', name: 'Bar widgets'}, {key: 'enabled', name: 'Enabled'}, {key: 'disabled', name: 'Disabled'}]
    readonly property var filtered: rows.filter(function(p) {
        var match = category === 'all' ? p.installed
            : category === 'user' ? p.installed && !p.firstParty
            : category === 'bar' ? p.widget
            : category === 'enabled' ? p.enabled === true
            : p.installed && p.enabled !== true;
        return match && [p.name, p.description, p.kinds.join(' ')].join(' ').toLowerCase().indexOf(query.toLowerCase()) >= 0;
    })
    readonly property var sectionWords: ({left: 'Left', center: 'Center', right: 'Right'})

    function load() {
        busy = true;
        SystemSettings.request({action: 'plugins', request: {action: 'list'}}, (result, failure) => {
            busy = false; loaded = true; error = failure;
            if (!failure) { rows = result.plugins || []; message = ''; }
        });
    }
    function act(request) {
        if (busy || !selected || selected.protected || !selected.installed) return;
        busy = true; error = '';
        SystemSettings.request({action: 'plugins', request: Object.assign({id: selected.id}, request)}, (result, failure) => {
            busy = false;
            if (failure) error = failure;
            else { rows = result.plugins || []; message = result.message || ''; }
        });
    }
    function kindLine(p) {
        return !p.installed ? 'Not installed' : (p.firstParty ? 'Built-in' : 'User plugin') + (p.section ? '  ·  ' + sectionWords[p.section] + ' bar' : '');
    }

    // Read afresh each time the page is opened; there is no refresh button.
    onActiveChanged: if (active && !busy) load()
    Connections {
        target: SystemSettings
        function onChanged(domain) { if (root.active && domain === 'plugins') root.load(); }
    }
    Shortcut { enabled: root.active; sequence: 'Ctrl+F'; onActivated: search.forceActiveFocus() }

    component Words: Text {
        textFormat: Text.PlainText
        color: Theme.text
        font.family: Theme.mono
        font.pixelSize: 13
        wrapMode: Text.WordWrap
    }
    component Quiet: Words { color: Theme.muted; font.pixelSize: 12 }

    ColumnLayout {
        anchors.fill: parent
        spacing: 16
        RowLayout {
            Layout.fillWidth: true
            ColumnLayout {
                spacing: 4
                Words { objectName: 'plugins-title'; text: 'Plugins'; font.pixelSize: 25; font.weight: Font.Bold; wrapMode: Text.NoWrap }
                Words {
                    objectName: 'plugins-counts'
                    text: root.loaded && !root.error ? root.installed.length + ' installed  ·  ' + root.enabledCount + ' enabled' : ' '
                    color: Theme.muted
                    wrapMode: Text.NoWrap
                }
            }
            Item { Layout.fillWidth: true }
        }
        Field {
            id: search
            objectName: 'plugins-search'
            Layout.fillWidth: true
            font.family: Theme.mono
            font.pixelSize: 13
            placeholderText: 'Search installed plugins…'
            Accessible.name: 'Search installed plugins'
            onTextChanged: root.query = text
            Keys.onEscapePressed: focus = false
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            Repeater {
                model: root.categories
                delegate: ManagerButton {
                    required property var modelData
                    objectName: 'plugins-tab-' + modelData.key
                    text: modelData.name
                    selected: root.category === modelData.key
                    onClicked: root.category = modelData.key
                }
            }
            Item { Layout.fillWidth: true }
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 20
            ListView {
                id: list
                objectName: 'plugins-list'
                Layout.fillWidth: true
                Layout.fillHeight: true
                Layout.minimumWidth: 0
                clip: true
                spacing: 6
                boundsBehavior: Flickable.StopAtBounds
                model: root.filtered
                ScrollBar.vertical: SScrollBar {}
                delegate: Rectangle {
                    id: row
                    required property var modelData
                    readonly property bool chosen: root.selectedId === modelData.id
                    objectName: 'plugin-row-' + modelData.id
                    width: list.width - 12
                    height: 66
                    radius: Theme.chromeRadius
                    color: chosen ? Theme.raised : mouse.containsMouse ? Theme.hover : 'transparent'
                    border.width: 1
                    border.color: chosen ? Theme.accent : Theme.line
                    MouseArea { id: mouse; anchors.fill: parent; hoverEnabled: true; onClicked: root.selectedId = row.modelData.id }
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        spacing: 12
                        Rectangle { implicitWidth: 7; implicitHeight: 7; radius: 4; color: row.modelData.enabled ? Theme.accent : Theme.muted }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 3
                            Words { Layout.fillWidth: true; text: row.modelData.name; font.weight: Font.Bold; elide: Text.ElideRight; wrapMode: Text.NoWrap }
                            Quiet { Layout.fillWidth: true; text: root.kindLine(row.modelData); elide: Text.ElideRight; wrapMode: Text.NoWrap }
                        }
                        ManagerButton {
                            objectName: 'plugin-toggle-' + row.modelData.id
                            visible: row.modelData.installed && !row.modelData.protected
                            text: row.modelData.enabled ? 'Disable' : 'Enable'
                            enabled: !root.busy
                            onClicked: { root.selectedId = row.modelData.id; root.act({action: row.modelData.enabled ? 'disable' : 'enable'}); }
                        }
                        Quiet { visible: row.modelData.installed && row.modelData.protected; text: 'Protected'; wrapMode: Text.NoWrap }
                    }
                }
                Quiet {
                    anchors.centerIn: parent
                    width: parent.width - 40
                    horizontalAlignment: Text.AlignHCenter
                    visible: root.loaded && list.count === 0
                    color: root.error ? Theme.dangerText : Theme.muted
                    text: root.error ? 'The bar could not be read. ' + root.error : root.rows.length ? 'No plugin matches.' : 'The bar reports no plugins.'
                }
            }
            Rectangle { Layout.fillHeight: true; implicitWidth: 1; color: Theme.line }
            ColumnLayout {
                objectName: 'plugin-pane'
                Layout.preferredWidth: 310; Layout.maximumWidth: 310
                Layout.fillHeight: true
                spacing: 16
                Words { objectName: 'plugin-pane-name'; Layout.fillWidth: true; text: root.selected ? root.selected.name : 'Your plugins, in one place'; font.pixelSize: 21; font.weight: Font.Bold }
                Words { Layout.fillWidth: true; text: root.selected ? (root.selected.description || 'No description.') : 'Select a plugin to see its details and its place on the bar.'; color: Theme.muted }
                Words {
                    objectName: 'plugin-pane-state'
                    Layout.fillWidth: true
                    visible: !!root.selected
                    text: !root.selected ? '' : !root.selected.installed ? 'Not installed' : (root.selected.enabled ? 'Enabled' : 'Disabled') + '  ·  ' + (root.selected.firstParty ? 'Built-in' : 'User plugin')
                    color: Theme.accentText
                }
                Quiet { Layout.fillWidth: true; visible: !!root.selected && root.selected.kinds.length > 0; text: root.selected ? root.selected.kinds.join('  ·  ') : '' }
                ManagerButton {
                    objectName: 'plugin-pane-toggle'
                    visible: !!root.selected && root.selected.installed && !root.selected.protected
                    text: root.selected && root.selected.enabled ? 'Disable' : 'Enable'
                    enabled: !root.busy
                    onClicked: root.act({action: root.selected.enabled ? 'disable' : 'enable'})
                }
                ColumnLayout {
                    objectName: 'plugin-placement'
                    Layout.fillWidth: true
                    visible: !!root.selected && root.selected.enabled === true && root.selected.widget && !root.selected.protected
                    spacing: 8
                    Quiet { text: 'BAR PLACEMENT'; font.pixelSize: 11; font.weight: Font.Bold }
                    RowLayout {
                        spacing: 6
                        Repeater {
                            model: ['left', 'center', 'right']
                            delegate: ManagerButton {
                                required property string modelData
                                objectName: 'plugin-section-' + modelData
                                text: root.sectionWords[modelData]
                                selected: !!root.selected && root.selected.section === modelData
                                enabled: !root.busy
                                onClicked: root.act({action: 'move', section: modelData})
                            }
                        }
                    }
                    RowLayout {
                        spacing: 6
                        ManagerButton { objectName: 'plugin-earlier'; text: '← Earlier'; enabled: !root.busy && !!root.selected && root.selected.position > 0; onClicked: root.act({action: 'move', section: root.selected.section, index: root.selected.position - 1}) }
                        ManagerButton { objectName: 'plugin-later'; text: 'Later →'; enabled: !root.busy && !!root.selected && root.selected.position >= 0 && root.selected.position < root.selected.sectionSize - 1; onClicked: root.act({action: 'move', section: root.selected.section, index: root.selected.position + 1}) }
                    }
                    Quiet { visible: !!root.selected && root.selected.position >= 0; text: root.selected ? 'Place ' + (root.selected.position + 1) + ' of ' + root.selected.sectionSize + ' in the ' + root.sectionWords[root.selected.section].toLowerCase() + ' section' : '' }
                }
                Quiet { Layout.fillWidth: true; visible: !!root.selected && root.selected.installed && root.selected.protected; text: 'Protected: kept so the bar and its manager stay reachable.' }
                Item { Layout.fillHeight: true }
            }
        }
        Words {
            objectName: 'plugins-status'
            Layout.fillWidth: true
            visible: text !== ''
            text: root.error && root.rows.length ? root.error : root.message
            color: root.error ? Theme.dangerText : Theme.muted
            font.pixelSize: 12
        }
    }
}
