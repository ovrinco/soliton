pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Settings › Processes: what runs, each with the reason it runs. A chosen
// process is ended on his press, bound to that exact process; the result is
// one word. Read when the tab opens; never on a clock.
Item {
    id: root
    objectName: 'processes-view'
    property bool active: false
    property var processes: []
    property bool loaded: false
    property bool reading: false
    property string error: ''
    property var selected: null
    property var result: null
    property bool stopping: false
    readonly property string query: search.text.trim().toLowerCase()
    readonly property var shown: !query ? processes : processes.filter(p =>
        (program(p) + ' ' + p.name + ' ' + reasonWords(p)).toLowerCase().indexOf(query) >= 0)
    // The engine's outcome, read as one word in its tone.
    readonly property var outcomes: ({
        'stopped': ['Ended', 'success'], 'exited': ['Gone', 'neutral'], 'stale': ['Gone', 'neutral'],
        'not-permitted': ['Refused', 'danger'], 'failed': ['Refused', 'danger'], 'still-running': ['Running', 'accent']
    })
    function program(p) { return p ? p.executable || p.name : ''; }
    // Plain words from the engine's: no process numbers, unit names, paths or closing stop.
    function plain(words) {
        return String(words || '').replace(/ \(pid \d+\)/g, '')
            .replace(/\[[^\]]*\]\s*/g, '').replace(/\S+\.(service|timer|socket|scope|slice|target|mount)\b\s*/g, '')
            .replace(/\s+(?:(?:for|in|at|on|of)\s+)?~?\/\S*/g, '').replace(/[.\s]+$/, '').replace(/[,;]?\s*within$/, '').trim();
    }
    function reasonWords(p) { return p && p.reason ? plain(p.reason.words) : ''; }
    function same(a, b) { return !!a && !!b && a.pid === b.pid && a.start_ticks === b.start_ticks; }
    function started(p) {
        if (!p || !p.started) return '';
        var when = new Date(p.started);
        return when.toDateString() === new Date().toDateString()
            ? Qt.formatTime(when, 'h:mm ap') : Qt.formatDateTime(when, 'MMM d, h:mm ap');
    }
    function memory(p) {
        if (!p || p.memory_kib === null || p.memory_kib === undefined) return '';
        var kib = p.memory_kib;
        return kib >= 1048576 ? (kib / 1048576).toFixed(1) + ' GB' : kib >= 1024 ? Math.round(kib / 1024) + ' MB' : kib + ' KB';
    }
    function load() {
        if (reading) return;
        reading = true;
        Backend.send('process-list', {}, (reply, ok, failure) => {
            reading = false;
            error = ok ? '' : (failure || 'Unreadable');
            loaded = ok;
            processes = ok && reply ? (reply.processes || []).slice().sort((a, b) => program(a).localeCompare(program(b))) : [];
            if (selected && !processes.some(p => same(p, selected))) { selected = null; result = null; }
        }, true);
    }
    function choose(p) {
        if (same(p, selected)) return;
        selected = p;
        result = null;
    }
    function stop(force) {
        if (!selected || stopping) return;
        var target = selected;
        stopping = true;
        Backend.send('process-stop', {pid: target.pid, start_ticks: target.start_ticks, signal: force ? 'kill' : 'term'}, (reply, ok) => {
            stopping = false;
            if (!same(target, selected)) return;
            result = ok && reply ? reply.outcome : 'failed';
            if (result === 'stopped' || result === 'exited' || result === 'stale')
                processes = processes.filter(p => !same(p, target));
        }, true);
    }
    onActiveChanged: if (active) load()

    RowLayout {
        anchors.fill: parent
        spacing: Theme.space16
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.space12
            SSearchField {
                id: search
                objectName: 'processes-search'
                scopeName: 'processes'
                Layout.preferredWidth: 320
            }
            ListView {
                id: list
                objectName: 'processes-list'
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: root.shown
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: SScrollBar {}
                delegate: AbstractButton {
                    id: row
                    required property var modelData
                    readonly property bool current: root.same(modelData, root.selected)
                    objectName: 'process-row'
                    width: ListView.view.width
                    implicitHeight: Theme.denseRowHeight
                    hoverEnabled: true
                    onClicked: root.choose(modelData)
                    Accessible.role: Accessible.ListItem
                    Accessible.name: root.program(modelData) + ', ' + root.reasonWords(modelData)
                    Accessible.selected: current
                    background: Rectangle {
                        color: row.current ? Theme.selection : row.hovered ? Theme.hover : 'transparent'
                        border.color: row.visualFocus ? Theme.accent : 'transparent'
                        border.width: 2
                        Rectangle { width: parent.width; height: 1; anchors.bottom: parent.bottom; color: Theme.line }
                    }
                    contentItem: RowLayout {
                        spacing: Theme.space12
                        LabelText {
                            Layout.leftMargin: Theme.space12
                            Layout.preferredWidth: 180
                            text: root.program(row.modelData)
                            font.weight: Font.Medium
                            elide: Text.ElideRight; wrapMode: Text.NoWrap
                        }
                        LabelText {
                            objectName: 'process-reason'
                            Layout.fillWidth: true
                            text: root.reasonWords(row.modelData)
                            color: row.modelData.reason && row.modelData.reason.known ? Theme.text : Theme.muted
                            font.pixelSize: Theme.fontSizeSmall
                            elide: Text.ElideRight; wrapMode: Text.NoWrap
                        }
                        LabelText {
                            Layout.preferredWidth: 80
                            text: row.modelData.user || ''
                            color: Theme.muted
                            font.pixelSize: Theme.fontSizeSmall
                            elide: Text.ElideRight; wrapMode: Text.NoWrap
                        }
                        LabelText {
                            Layout.preferredWidth: 110
                            Layout.rightMargin: Theme.space12
                            text: root.started(row.modelData)
                            color: Theme.muted
                            font.pixelSize: Theme.fontSizeSmall
                            horizontalAlignment: Text.AlignRight
                            elide: Text.ElideRight; wrapMode: Text.NoWrap
                        }
                    }
                }
                StateView {
                    anchors.fill: parent
                    visible: list.count === 0 && (root.loaded || !!root.error)
                    error: !!root.error
                    title: root.error ? 'Unreadable' : 'No match'
                }
            }
        }
        Rectangle {
            objectName: 'process-facts'
            visible: !!root.selected
            Layout.preferredWidth: 340
            Layout.fillHeight: true
            color: Theme.panel
            border.color: Theme.line
            radius: Theme.cornerRadius
            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.space16
                spacing: Theme.space8
                LabelText {
                    objectName: 'process-selected-name'
                    Layout.fillWidth: true
                    text: root.program(root.selected)
                    font.pixelSize: Theme.sectionTitleSize
                    font.weight: Font.DemiBold
                    elide: Text.ElideRight
                }
                LabelText {
                    objectName: 'process-selected-reason'
                    Layout.fillWidth: true
                    text: root.reasonWords(root.selected)
                    color: root.selected && root.selected.reason && root.selected.reason.known ? Theme.text : Theme.muted
                    wrapMode: Text.Wrap; elide: Text.ElideNone
                }
                Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; Layout.topMargin: Theme.space4; color: Theme.line }
                GridLayout {
                    Layout.fillWidth: true
                    columns: 2
                    columnSpacing: Theme.space12
                    rowSpacing: Theme.space4
                    Repeater {
                        model: !root.selected ? [] : [
                            ['Name', root.selected.name !== root.program(root.selected) ? root.selected.name : ''],
                            ['User', root.selected.user || ''],
                            ['Started', root.started(root.selected)],
                            ['Memory', root.memory(root.selected)],
                            ['Parent', root.selected.parent && root.selected.parent.name || ''],
                            ['Service', root.plain(root.selected.unit_description)]
                        ].filter(fact => !!fact[1])
                        delegate: RowLayout {
                            id: fact
                            required property var modelData
                            Layout.columnSpan: 2
                            Layout.fillWidth: true
                            LabelText { text: fact.modelData[0]; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall; Layout.preferredWidth: 70 }
                            LabelText { text: fact.modelData[1]; Layout.fillWidth: true; wrapMode: Text.Wrap; elide: Text.ElideNone }
                        }
                    }
                }
                RowLayout {
                    Layout.topMargin: Theme.space8
                    spacing: Theme.space12
                    SButton {
                        objectName: 'process-end'
                        text: 'End'
                        destructive: true
                        busy: root.stopping
                        visible: !!root.selected && root.selected.can_stop && root.result !== 'still-running'
                            && ['stopped', 'exited', 'stale'].indexOf(root.result) < 0
                        enabled: !root.stopping && Backend.available
                        onClicked: root.stop(false)
                    }
                    SButton {
                        objectName: 'process-force-end'
                        text: 'Force end'
                        destructive: true
                        busy: root.stopping
                        visible: root.result === 'still-running'
                        enabled: !root.stopping && Backend.available
                        onClicked: root.stop(true)
                    }
                    SStateWord {
                        objectName: 'process-result'
                        visible: !!root.result
                        word: root.result ? (root.outcomes[root.result] || ['Refused'])[0] : ''
                        tone: root.result ? (root.outcomes[root.result] || ['', 'danger'])[1] : 'neutral'
                    }
                }
                Item { Layout.fillHeight: true }
            }
        }
    }
}
