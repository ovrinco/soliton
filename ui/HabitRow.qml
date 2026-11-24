import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// One habit on the Habits page. A habit to form toggles from its face; a habit
// to break shows the day's count with minus and plus. Dragging the face asks
// the page to reorder; the page decides where it lands.
Item {
    id: row
    property var habit: ({})
    property var value: null
    property bool focusedRow: false
    property bool draggable: true
    readonly property bool breaking: habit.direction === 'break'
    readonly property bool done: !breaking && value === 1
    readonly property int count: breaking && value !== null && value !== undefined ? Math.round(value) : 0
    readonly property bool dragging: face.dragging
    signal activated()
    signal dragMoved(real sceneY, real sceneX)
    signal dragDropped(real sceneY, real sceneX)

    implicitHeight: Theme.denseRowHeight
    objectName: 'habit-row-' + (habit.id || '')

    Rectangle {
        anchors.fill: parent
        color: face.dragging ? Theme.raised : face.containsMouse ? Theme.hover : 'transparent'
        radius: Theme.chromeRadius
        border.width: row.focusedRow ? 2 : 0
        border.color: Theme.accent
    }

    MouseArea {
        id: face
        property real pressY: 0
        property bool dragging: false
        anchors.fill: parent
        hoverEnabled: true
        cursorShape: row.breaking ? Qt.ArrowCursor : Qt.PointingHandCursor
        onPressed: mouse => { pressY = mouse.y; dragging = false; row.activated(); }
        onPositionChanged: mouse => {
            if (!pressed) return;
            if (!dragging && row.draggable && Math.abs(mouse.y - pressY) > 6) dragging = true;
            if (dragging) {
                var p = mapToItem(null, mouse.x, mouse.y);
                row.dragMoved(p.y, p.x);
            }
        }
        onReleased: mouse => {
            if (dragging) {
                dragging = false;
                var p = mapToItem(null, mouse.x, mouse.y);
                row.dragDropped(p.y, p.x);
            } else if (!row.breaking) {
                Backend.habitCheckIn(row.habit.id, row.done ? 0 : 1);
            }
        }
        onCanceled: dragging = false
    }

    RowLayout {
        anchors.fill: parent; anchors.leftMargin: 8; anchors.rightMargin: 4; spacing: 10

        Item {
            visible: !row.breaking
            implicitWidth: 16; implicitHeight: 16
            // Empty: the house check box outline. Done: filled in the accent,
            // the mark in the accent's readable foreground.
            Rectangle {
                anchors.fill: parent
                radius: Theme.chromeRadius
                color: row.done ? Theme.accent : 'transparent'
                border.width: row.done ? 0 : 1.5
                border.color: Theme.muted
            }
            SIcon { anchors.centerIn: parent; size: 12; name: 'check'; color: Theme.accentForeground; visible: row.done }
        }
        LabelText {
            Layout.fillWidth: !row.breaking
            Layout.maximumWidth: row.breaking ? Math.max(60, row.width - 220) : -1
            text: row.habit.title || ''
            font.pixelSize: 14
        }
        RowLayout {
            visible: row.breaking
            spacing: 2
            LabelText {
                Layout.leftMargin: 12
                Layout.preferredWidth: 28
                horizontalAlignment: Text.AlignRight
                text: String(row.count)
                font.family: Theme.mono; font.pixelSize: 14
            }
            AbstractButton {
                id: minus
                implicitWidth: 24; implicitHeight: 24
                enabled: row.count > 0
                hoverEnabled: true
                focusPolicy: Qt.NoFocus
                onClicked: Backend.habitCheckIn(row.habit.id, row.count - 1)
                contentItem: Text {
                    text: '−'
                    font.family: Theme.font; font.pixelSize: 16
                    color: minus.enabled ? Theme.text : Theme.muted
                    horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
                }
                background: Rectangle { radius: Theme.chromeRadius; color: minus.hovered ? Theme.hover : 'transparent' }
                Accessible.role: Accessible.Button
                Accessible.name: 'Take one away'
            }
            AbstractButton {
                id: plus
                implicitWidth: 24; implicitHeight: 24
                hoverEnabled: true
                focusPolicy: Qt.NoFocus
                onClicked: Backend.habitCheckIn(row.habit.id, row.count + 1)
                contentItem: Item { SIcon { anchors.centerIn: parent; size: 16; name: 'plus'; color: Theme.text } }
                background: Rectangle { radius: Theme.chromeRadius; color: plus.hovered ? Theme.hover : 'transparent' }
                Accessible.role: Accessible.Button
                Accessible.name: 'Add one'
            }
            LabelText {
                Layout.leftMargin: 8
                text: 'usually ' + (row.habit.baseline === undefined ? '' : row.habit.baseline)
                font.pixelSize: 12
                color: Theme.muted
            }
        }
        Item { Layout.fillWidth: row.breaking }
        Item {
            implicitWidth: 28; implicitHeight: 28
        SButton {
            id: moreButton
            anchors.fill: parent
            iconName: 'more'; accessibleLabel: 'More actions'; quiet: true
            focusPolicy: Qt.NoFocus
            visible: face.containsMouse || hovered || row.focusedRow || menu.visible
            onClicked: menu.open()
        }
        }
    }

    SPopup {
        id: menu
        trigger: moreButton
        alignRight: true
        parent: Overlay.overlay
        width: 220
        padding: 6
        contentItem: ColumnLayout {
            spacing: 2
            MenuAction { text: 'Edit'; onClicked: { menu.close(); Backend.openHabitForm(row.habit.id); } }
            MenuAction { visible: !!row.habit.restorable; text: 'Restore check-in'; onClicked: { menu.close(); Backend.habitRestore(row.habit.id); } }
            MenuAction { visible: row.habit.bundle !== 'morning'; text: 'Move to morning'; onClicked: { menu.close(); Backend.habitMove(row.habit.id, 'morning'); } }
            MenuAction { visible: row.habit.bundle !== 'evening'; text: 'Move to evening'; onClicked: { menu.close(); Backend.habitMove(row.habit.id, 'evening'); } }
            MenuAction { visible: !!row.habit.bundle; text: 'Move to through the day'; onClicked: { menu.close(); Backend.habitMove(row.habit.id, null); } }
            MenuAction { text: row.habit.private === true ? 'Unmark private' : 'Mark private'; onClicked: { menu.close(); Backend.habitSetPrivate(row.habit.id, row.habit.private !== true); } }
        }
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: habit.title || ''
    Accessible.checked: done
}
