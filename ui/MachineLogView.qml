pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Settings › Log: what the machine did, one short line per kind of event,
// grouped by day. Repeats arrive folded with their count and open to their
// times. Read when the tab opens and when the filter changes; never on a clock.
Item {
    id: root
    objectName: 'machine-log-view'
    property bool active: false
    property var days: []
    property bool loaded: false
    property bool reading: false
    property string error: ''
    property var expanded: ({})
    // All, Mine (his launches) or Machine (timers, sockets, updates, power).
    readonly property string filter: ['All', 'Mine', 'Machine'].indexOf(Backend.prefs.machineLogFilter) >= 0 ? Backend.prefs.machineLogFilter : 'All'
    readonly property var actorOf: ({Mine: 'his', Machine: 'machine'})
    readonly property var categoryColors: ({
        Updates: Theme.accent, Power: Theme.danger, Scheduled: Theme.edge,
        Started: Theme.green, Stopped: Theme.muted, 'Signed in': Theme.text
    })
    readonly property var rows: {
        var out = [];
        var actor = actorOf[filter];
        days.slice().reverse().forEach(day => {
            day.lines.filter(line => !actor || line.actor === actor).forEach((line, index) => {
                out.push({date: day.date, key: day.date + '|' + index + '|' + line.words, line: line});
            });
        });
        return out;
    }
    readonly property var categories: {
        var seen = [];
        rows.forEach(row => { if (seen.indexOf(row.line.category) < 0) seen.push(row.line.category); });
        return seen;
    }
    function isoDay(date) { return Qt.formatDate(date, 'yyyy-MM-dd'); }
    function load() {
        if (reading) return;
        var last = new Date();
        var first = new Date(last.getFullYear(), last.getMonth(), last.getDate() - 6);
        reading = true;
        Backend.send('machine-log', {first: isoDay(first), last: isoDay(last)}, (result, ok, failure) => {
            reading = false;
            error = ok ? '' : (failure || 'Unreadable');
            loaded = ok;
            days = ok && result ? result.days || [] : [];
            expanded = ({});
        }, true);
    }
    function choose(name) {
        if (name === filter) return;
        Backend.setting('machineLogFilter', name);
        load();
    }
    function toggle(key) {
        var next = Object.assign({}, expanded);
        if (next[key]) delete next[key]; else next[key] = true;
        expanded = next;
    }
    function dayHeading(date) {
        var parts = date.split('-');
        var day = new Date(Number(parts[0]), Number(parts[1]) - 1, Number(parts[2]));
        var today = new Date();
        var yesterday = new Date(today.getFullYear(), today.getMonth(), today.getDate() - 1);
        if (date === isoDay(today)) return 'Today';
        if (date === isoDay(yesterday)) return 'Yesterday';
        return Qt.formatDate(day, 'dddd, MMM d');
    }
    function clock(stamp) { return Qt.formatTime(new Date(stamp), 'h:mm ap'); }
    onActiveChanged: if (active) load()

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.space12
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.space8
            Repeater {
                model: ['All', 'Mine', 'Machine']
                delegate: SButton {
                    required property string modelData
                    objectName: 'machine-log-filter-' + modelData.toLowerCase()
                    text: modelData
                    primary: root.filter === modelData
                    onClicked: root.choose(modelData)
                }
            }
            Item { Layout.fillWidth: true }
            Repeater {
                model: root.categories
                delegate: RowLayout {
                    id: legend
                    required property string modelData
                    spacing: Theme.space4
                    Rectangle { implicitWidth: 8; implicitHeight: 8; radius: 4; color: root.categoryColors[legend.modelData] || Theme.muted }
                    LabelText { text: legend.modelData; color: Theme.muted; font.pixelSize: Theme.fontSizeSmall }
                }
            }
        }
        ListView {
            id: list
            objectName: 'machine-log-list'
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: root.rows
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar {}
            section.property: 'date'
            section.criteria: ViewSection.FullString
            section.delegate: Item {
                id: heading
                required property string section
                width: ListView.view.width
                implicitHeight: 40
                LabelText {
                    objectName: 'machine-log-day'
                    anchors.left: parent.left; anchors.leftMargin: Theme.space12
                    anchors.bottom: parent.bottom; anchors.bottomMargin: Theme.space8
                    text: root.dayHeading(heading.section)
                    font.weight: Font.DemiBold
                }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
            }
            delegate: ColumnLayout {
                id: entry
                required property var modelData
                readonly property var line: modelData.line
                readonly property bool folded: line.count > 1
                readonly property bool open: !!root.expanded[modelData.key]
                width: ListView.view.width
                spacing: 0
                AbstractButton {
                    id: lineButton
                    objectName: entry.folded ? 'machine-log-folded' : 'machine-log-line'
                    Layout.fillWidth: true
                    implicitHeight: Theme.denseRowHeight
                    hoverEnabled: true
                    focusPolicy: entry.folded ? Qt.StrongFocus : Qt.NoFocus
                    onClicked: if (entry.folded) root.toggle(entry.modelData.key)
                    Accessible.role: entry.folded ? Accessible.Button : Accessible.ListItem
                    Accessible.name: entry.line.category + ', ' + entry.line.words + (entry.folded ? ', ' + entry.line.count : '')
                    background: Rectangle {
                        color: lineButton.hovered && entry.folded ? Theme.hover : 'transparent'
                        border.color: lineButton.visualFocus ? Theme.accent : 'transparent'
                        border.width: 2
                    }
                    contentItem: RowLayout {
                        spacing: Theme.space8
                        Rectangle {
                            Layout.leftMargin: Theme.space12
                            implicitWidth: 3; implicitHeight: 16; radius: 1
                            color: root.categoryColors[entry.line.category] || Theme.muted
                        }
                        LabelText {
                            Layout.fillWidth: true
                            text: entry.line.words
                            elide: Text.ElideRight
                            wrapMode: Text.NoWrap
                        }
                        LabelText {
                            objectName: 'machine-log-count'
                            visible: entry.folded
                            text: String(entry.line.count)
                            color: Theme.muted
                            font.pixelSize: Theme.fontSizeSmall
                            font.weight: Font.Medium
                        }
                        SIcon {
                            Layout.rightMargin: Theme.space12
                            name: entry.open ? 'down' : 'right'
                            color: Theme.muted
                            implicitWidth: 16; implicitHeight: 16
                            opacity: entry.folded ? 1 : 0
                        }
                    }
                }
                Flow {
                    objectName: 'machine-log-times'
                    visible: entry.open
                    Layout.fillWidth: true
                    Layout.leftMargin: Theme.space24 + Theme.space4
                    Layout.rightMargin: Theme.space12
                    Layout.bottomMargin: Theme.space8
                    spacing: Theme.space12
                    Repeater {
                        model: entry.open ? entry.line.times : []
                        delegate: LabelText {
                            required property string modelData
                            text: root.clock(modelData)
                            color: Theme.muted
                            font.pixelSize: Theme.fontSizeSmall
                            font.family: Theme.mono
                        }
                    }
                }
            }
            StateView {
                anchors.fill: parent
                visible: list.count === 0 && (root.loaded || !!root.error)
                error: !!root.error
                title: root.error ? 'Unreadable' : 'Nothing recorded'
            }
        }
    }
}
