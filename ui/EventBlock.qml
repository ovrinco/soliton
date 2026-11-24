import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

Rectangle {
    id: block
    objectName: "event-block"
    property var record: entry.event

    property real baseX: entry.column * columnWidth / entry.columns + 2
    property real baseY: (entry.start - startMinute) * hourHeight / 60
    required property real columnWidth
    required property string day
    required property var entry
    property color eventColor: entry.event.color
    required property real hourHeight
    property real moveX: 0
    property real moveY: 0
    property real originX: 0
    property real originY: 0
    property real resizeY: 0
    required property int startMinute
    readonly property bool proposed: !!entry.event.proposal_id

    Hint {
        text: (block.proposed ? '◇ Proposed · ' : '') + entry.event.title + ' · ' + entry.event.local_start.replace('T', ' ') + ' → ' + entry.event.local_end.replace('T', ' ')
        visible: drag.containsMouse && !drag.pressed && !resizer.pressed
    }
    border.color: proposed ? Theme.accent : Backend.selectedId === entry.event.id ? Theme.accent : Qt.darker(entry.event.color, 1.5)
    border.width: proposed ? 2 : 1
    color: proposed ? Theme.panel : Qt.tint(Theme.panel, Qt.rgba(eventColor.r, eventColor.g, eventColor.b, .18))
    height: Math.max(3, (entry.end - entry.start) * hourHeight / 60 + resizeY)
    opacity: entry.event.status === 'completed' ? .55 : 1
    radius: Theme.cornerRadius
    width: Math.max(24, columnWidth / entry.columns - 4)
    x: baseX + moveX
    y: baseY + moveY
    z: drag.pressed || resizer.pressed ? 40 : 2

    // Keep the drag attached to the moving block.  Attaching it to the
    // child MouseArea leaves Qt's drag geometry at the original coordinates
    // while the block moves under the pointer, which can miss real targets.
    Drag.active: drag.dragActive
    Drag.hotSpot.x: drag.grabPoint.x
    Drag.hotSpot.y: drag.grabPoint.y
    Drag.keys: ['calendar-event']
    Drag.source: drag

    Rectangle {
        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.top: parent.top
        color: entry.event.color
        radius: 2
        width: 3
    }
    Column {
        anchors.bottomMargin: block.height < 26 ? 0 : 6
        anchors.fill: parent
        anchors.leftMargin: 9
        anchors.rightMargin: 5
        anchors.topMargin: block.height < 26 ? 0 : 5
        clip: true
        spacing: 3

        LabelText {
            font.pixelSize: block.height < 26 ? 10 : 12
            font.strikeout: block.entry.event.status === 'completed'
            font.weight: Font.Medium
            maximumLineCount: block.height > 70 ? 2 : 1
            text: (block.proposed ? '◇ Proposed · ' : '') + block.entry.event.title
            width: parent.width
            wrapMode: Text.Wrap
        }
        LabelText {
            color: Theme.muted
            font.pixelSize: 10
            text: (block.entry.event.local_start.substring(0, 10) < block.day ? 'Continues' : block.entry.event.local_start.substring(11)) + ' · ' + (block.entry.event.schedule.duration_minutes) + 'm' + (block.entry.event.occurrence ? ' ↻' : '')
            visible: block.height > 38
            width: parent.width
        }
        LabelText {
            color: block.entry.event.color
            font.pixelSize: 10
            text: block.entry.event.goal_title || block.entry.event.category_title || ''
            visible: block.height > 75
            width: parent.width
        }
    }
    MouseArea {
        id: drag

        property bool dropHandled: false
        property bool dragActive: false
        // Calendar drops use the pointer hotspot so the slot follows the
        // grabbed point, while board/card drops use the pointer directly.
        readonly property bool calendarDrag: true
        property point grabPoint: Qt.point(0, 0)
        property bool moved: false
        readonly property var record: block.entry.event

        anchors.fill: parent
        cursorShape: block.proposed ? Qt.PointingHandCursor : pressed ? Qt.ClosedHandCursor : Qt.OpenHandCursor
        hoverEnabled: true
        preventStealing: true

        onCanceled: {
            block.moveX = 0;
            block.moveY = 0;
            drag.dragActive = false;
            drag.dropHandled = false;
            moved = false;
        }
        onClicked: if (!moved) {
            if (block.proposed)
                Backend.reviewProposalRequested(block.entry.event.proposal_id);
            else
                Backend.select(block.entry.event.id, block.entry.event);
        }
        onDoubleClicked: {
            if (block.proposed) {
                Backend.reviewProposalRequested(block.entry.event.proposal_id);
                return;
            }
            Backend.select(block.entry.event.id, block.entry.event);
            Backend.detailFull = true;
        }
        onPositionChanged: mouse => {
            if (block.proposed)
                return;
            if (pressed) {
                var p = mapToItem(null, mouse.x, mouse.y);
                var dx = p.x - block.originX, dy = p.y - block.originY;
                if (moved || Math.abs(dx) + Math.abs(dy) > 7) {
                    moved = true;
                    dragActive = true;
                    block.moveX = dx;
                    block.moveY = dy;
                }
            }
        }
        onPressed: mouse => {
            forceActiveFocus();
            if (block.proposed)
                return;
            var p = mapToItem(null, mouse.x, mouse.y);
            moved = false;
            dropHandled = false;
            dragActive = false;
            grabPoint = Qt.point(mouse.x, mouse.y);
            block.originX = p.x;
            block.originY = p.y;
        }
        onReleased: {
            if (moved)
                block.Drag.drop();
            var handled = dropHandled;
            dragActive = false;
            if (handled) {
                block.moveX = 0;
                block.moveY = 0;
                return;
            }
            // A drag released outside an accepted DropArea is a no-op.
            block.moveX = 0;
            block.moveY = 0;
        }

        Keys.onEscapePressed: event => {
            if (dragActive) {
                block.moveX = 0;
                block.moveY = 0;
                dragActive = false;
                dropHandled = false;
                moved = false;
                event.accepted = true;
            } else event.accepted = false;
        }
    }
    MouseArea {
        id: resizer
        property bool cancelled: false
        visible: !block.proposed

        anchors.bottom: parent.bottom
        anchors.left: parent.left
        anchors.right: parent.right
        cursorShape: Qt.SizeVerCursor
        height: Math.min(7, block.height / 3)

        onCanceled: { cancelled = true; block.resizeY = 0; }
        onPositionChanged: mouse => {
            if (pressed && !cancelled)
                block.resizeY = mapToItem(null, mouse.x, mouse.y).y - block.originY;
        }
        onPressed: mouse => {
            forceActiveFocus();
            cancelled = false;
            block.originY = mapToItem(null, mouse.x, mouse.y).y;
        }
        Keys.onEscapePressed: event => {
            if (pressed) {
                cancelled = true;
                block.resizeY = 0;
                event.accepted = true;
            } else event.accepted = false;
        }
        onReleased: {
            if (cancelled) { block.resizeY = 0; return; }
            var delta = Math.round(block.resizeY / block.hourHeight * 60 / 15) * 15;
            if (!delta) { block.resizeY = 0; return; }
            var duration = Math.max(15, block.entry.event.schedule.duration_minutes + delta);
            Backend.moveEvent(block.entry.event, block.entry.event.local_start.substring(0, 10), Cal.minutes(block.entry.event.local_start), duration);
            block.resizeY = 0;
        }
    }
}
