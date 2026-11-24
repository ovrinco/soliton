import QtQuick
import QtQuick.Layouts
import "CalendarMath.js" as Cal

// Past weeks: one row per habit, one 12 px cell per day, habits to form above
// the baseline and habits to break below it. The cell never shrinks; the span
// is the whole weeks that fit, ending on the day shown.
ColumnLayout {
    id: map
    property var formIds: []
    property var breakIds: []
    readonly property int cell: 12
    readonly property int pitch: 14
    readonly property int titleWidth: 140
    readonly property int weeks: Math.max(4, Math.min(52, Math.floor((width - titleWidth - 12) / (7 * pitch))))
    readonly property var rows: {
        var byId = {};
        Backend.habitMap.habits.forEach(h => byId[h.id] = h);
        return {form: formIds.map(id => byId[id]).filter(h => !!h), brk: breakIds.map(id => byId[id]).filter(h => !!h)};
    }
    readonly property var days: {
        var through = Backend.habitShownDay, list = [];
        for (var i = Backend.habitSpan - 1; i >= 0; --i) list.push(Cal.add(through, -i));
        return list;
    }
    spacing: 4

    onWeeksChanged: spanTimer.restart()
    Component.onCompleted: spanTimer.restart()
    Timer { id: spanTimer; interval: 120; onTriggered: if (map.width > 0) Backend.habitSpan = map.weeks * 7 }

    function longDay(day) {
        var p = day.split('-');
        return new Date(Number(p[0]), Number(p[1]) - 1, Number(p[2])).toLocaleDateString(Qt.locale('en_GB'), 'dddd d MMMM');
    }
    function shortMonth(day) {
        var p = day.split('-');
        return new Date(Number(p[0]), Number(p[1]) - 1, 1).toLocaleDateString(Qt.locale('en_GB'), 'MMM');
    }
    function cellColour(h, c) {
        if (!c || c.value === null || c.value === undefined) return 'transparent';
        if (h.direction !== 'break') return c.value >= 1 ? Theme.green : Theme.raised;
        if (c.against !== 'above') return Theme.raised;
        var over = Math.ceil(c.value - h.baseline);
        return Qt.alpha(Theme.danger, over >= 3 ? 1 : over === 2 ? .7 : .45);
    }
    function words(h, c) {
        var head = h.title + ' · ' + longDay(c.day) + ' · ';
        if (c.value === null || c.value === undefined) return head + 'no entry';
        if (h.direction !== 'break') return head + (c.value >= 1 ? 'done' : 'not done');
        var n = Math.round(c.value);
        return head + n + (n === 1 ? ' time' : ' times') + ', usually ' + h.baseline;
    }

    RowLayout {
        Layout.fillWidth: true
        LabelText {
            Layout.fillWidth: true
            text: 'Past weeks'
            font.pixelSize: Theme.sectionTitleSize; font.weight: Font.Medium
        }
        LabelText {
            text: map.weeks + ' weeks to ' + (Backend.habitShownDay === Backend.today ? 'today' : map.longDay(Backend.habitShownDay))
            color: Theme.muted
        }
    }

    // Month names above the first cell of each month, where they fit.
    Item {
        Layout.fillWidth: true
        Layout.topMargin: 6
        implicitHeight: 16
        Repeater {
            model: {
                var marks = [], last = -100;
                map.days.forEach((d, i) => {
                    if (d.substring(8) === '01' && i * map.pitch - last >= 32) {
                        marks.push({x: i * map.pitch, text: map.shortMonth(d)});
                        last = i * map.pitch;
                    }
                });
                return marks;
            }
            delegate: LabelText {
                required property var modelData
                x: map.titleWidth + 12 + modelData.x
                text: modelData.text
                font.pixelSize: 12
                color: Theme.muted
            }
        }
    }

    component MapRow: Item {
        id: mapRow
        property var habit: ({})
        Layout.fillWidth: true
        implicitHeight: map.cell + 4
        LabelText {
            width: map.titleWidth
            anchors.verticalCenter: parent.verticalCenter
            text: mapRow.habit.title
            font.pixelSize: 13
        }
        Item {
            id: cells
            x: map.titleWidth + 12
            width: map.days.length * map.pitch
            height: parent.height
            Repeater {
                model: mapRow.habit.days || []
                delegate: Rectangle {
                    required property var modelData
                    required property int index
                    x: index * map.pitch
                    y: 2
                    width: map.cell; height: map.cell
                    radius: Theme.chromeRadius
                    color: map.cellColour(mapRow.habit, modelData)
                }
            }
            MouseArea {
                id: hover
                anchors.fill: parent
                hoverEnabled: true
                acceptedButtons: Qt.NoButton
                property int at: -1
                onPositionChanged: mouse => {
                    var i = Math.floor(mouse.x / map.pitch);
                    var cellsOf = mapRow.habit.days || [];
                    if (i === at || i < 0 || i >= cellsOf.length) return;
                    at = i;
                    hint.close();
                    hint.text = map.words(mapRow.habit, cellsOf[i]);
                    var p = cells.mapToItem(map, i * map.pitch, 0);
                    hint.x = Math.min(p.x, map.width - 360);
                    hint.y = p.y - 34;
                    hint.open();
                }
                onExited: { at = -1; hint.close(); }
            }
        }
    }

    Repeater {
        model: map.rows.form
        delegate: MapRow { required property var modelData; habit: modelData }
    }
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: 3; Layout.bottomMargin: 3
        implicitHeight: 1
        color: Theme.line
        visible: map.rows.form.length + map.rows.brk.length > 0
    }
    Repeater {
        model: map.rows.brk
        delegate: MapRow { required property var modelData; habit: modelData }
    }

    Hint {
        id: hint
        parent: map
        delay: 600
        timeout: -1
    }
}
