import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "CalendarMath.js" as Cal

Item {
    id: card
    objectName: "task-card"

    property bool compact: false
    readonly property string currentStatus: Cal.effectiveStatus(Backend, record) || 'todo'
    property bool done: currentStatus === 'completed'
    property var dragRecord: record
    property bool draggable: true
    property bool selectable: false
    property bool selected: false
    // A page that keeps the item recoverable (Trash, with Undo) offers the quick delete.
    property bool deletable: false
    signal selectionToggled(bool shiftKey)
    signal deleteRequested()
    required property var record
    // Who owns the task, kept as an owner:human or owner:agent tag; untagged means the user.
    readonly property string owner: (record.tags || []).indexOf('owner:agent') >= 0 ? 'agent' : 'human'
    function setOwner(owner) {
        var tags = (card.record.tags || []).filter(t => t.indexOf('owner:') !== 0).concat(['owner:' + owner]);
        Backend.mutate(card.record, {tags: tags});
    }

    // A task completed while its card is on screen keeps its place until its
    // view is left, so the cards around it never move.
    readonly property var completionReceipt: Backend.prefs.lastTaskCompletion || null
    function holdCompleted() {
        var receipt = Backend.prefs.lastTaskCompletion || null, now = Date.now();
        if (visible && receipt && receipt.graceUntil > now && receipt.graceUntil < now + 5000
                && Cal.receiptEntries(Backend).some(e => e.id === card.record.id))
            Backend.setting('lastTaskCompletion', Object.assign({}, receipt, {graceUntil: now + 600000}));
    }
    // Deferred: the hold rewrites the receipt this handler reads.
    onCompletionReceiptChanged: Qt.callLater(card.holdCompleted)
    Component.onCompleted: Qt.callLater(card.holdCompleted)
    Connections {
        target: Backend
        function onRouteLeaving() {
            var receipt = Backend.prefs.lastTaskCompletion || {};
            if (receipt.graceUntil > Date.now() + 5000 && Cal.receiptEntries(Backend).some(e => e.id === card.record.id))
                Backend.setting('lastTaskCompletion', Object.assign({}, receipt, {graceUntil: 0}));
        }
    }

    implicitHeight: compact ? 62 : 100

    Rectangle {
        anchors.fill: parent
        border.color: card.selected ? Theme.accent : Backend.selectedId === card.record.id ? Theme.accent : Theme.line
        border.width: card.selected ? 2 : 1
        color: card.selected ? Qt.tint(Theme.selection, Qt.rgba(Theme.accent.r, Theme.accent.g, Theme.accent.b, .15)) : Backend.selectedId === card.record.id ? Theme.selection : mouse.containsMouse ? Theme.hover : Theme.raised
        opacity: card.done ? .65 : 1
        radius: Theme.cornerRadius

        Rectangle {
            anchors.bottom: parent.bottom
            anchors.left: parent.left
            anchors.margins: 1
            anchors.top: parent.top
            color: card.record.color || Theme.accent
            radius: 2
            width: 3
        }
        CardDrag {
            id: mouse

            card: card
            draggable: card.draggable
            record: card.record

            onActivated: {
                Backend.select(card.record.id);
            }
            onOpened: {
                Backend.select(card.record.id);
                Backend.detailFull = true;
            }
        }
        RowLayout {
            anchors.fill: parent
            anchors.margins: 10
            spacing: 9

            Item {
                Layout.preferredWidth: 32
                Layout.preferredHeight: 32
                Rectangle {
                id: selectBox
                anchors.centerIn: parent
                visible: card.selectable
                enabled: card.selectable
                activeFocusOnTab: card.selectable
                Accessible.role: Accessible.CheckBox
                Accessible.name: 'Select ' + card.record.title
                Accessible.checked: card.selected
                Accessible.ignored: !card.selectable
                Keys.onSpacePressed: event => { card.selectionToggled(!!(event.modifiers & Qt.ShiftModifier)); event.accepted = true; }
                Keys.onReturnPressed: event => { card.selectionToggled(!!(event.modifiers & Qt.ShiftModifier)); event.accepted = true; }
                width: 16
                height: 16
                radius: 1
                border.color: activeFocus || card.selected ? Theme.accent : Theme.muted
                border.width: activeFocus || card.selected ? 2 : 1
                color: card.selected ? Theme.accent : 'transparent'
                LabelText {
                    anchors.centerIn: parent
                    text: '✓'
                    font.pixelSize: 10
                    color: Theme.accentForeground
                    visible: card.selected
                }
                MouseArea {
                    anchors.fill: parent
                    anchors.topMargin: -9; anchors.bottomMargin: -9
                    anchors.leftMargin: -4; anchors.rightMargin: -4
                    cursorShape: Qt.PointingHandCursor
                    onClicked: mouse => {
                        selectBox.forceActiveFocus();
                        card.selectionToggled(!!(mouse.modifiers & Qt.ShiftModifier));
                    }
                }
                }
                SButton {
                    id: statusButton
                    anchors.fill: parent
                    visible: !card.selectable
                    Accessible.name: 'Set status for ' + card.record.title
                    implicitWidth: 32
                    implicitHeight: 32
                    quiet: true
                    text: card.done ? '✓' : card.currentStatus === 'in_progress' ? '◐' : '○'
                    onClicked: statusMenu.open()
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 6

                LabelText {
                    Layout.fillWidth: true
                    elide: Text.ElideRight
                    font.pixelSize: 13
                    font.strikeout: card.done
                    font.weight: Font.Medium
                    maximumLineCount: card.compact ? 1 : 2
                    text: card.record.title
                    wrapMode: Text.Wrap
                }
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 8

                    LabelText {
                        color: card.record.priority === 'high' ? Theme.danger : Theme.muted
                        font.pixelSize: 10
                        text: card.record.priority === 'high' ? 'HIGH' : card.record.priority === 'low' ? 'Low' : 'Medium'
                    }
                    LabelText {
                        id: ownerLabel
                        objectName: 'task-owner'
                        color: card.owner === 'agent' ? Theme.accentText : Theme.muted
                        font.pixelSize: 10
                        text: card.owner === 'agent' ? 'Agent' : 'Human'
                        Accessible.role: Accessible.Button
                        Accessible.name: 'Owner of ' + card.record.title + ': ' + text
                        MouseArea {
                            anchors.fill: parent
                            anchors.margins: -4
                            enabled: !card.selectable
                            cursorShape: Qt.PointingHandCursor
                            onClicked: ownerMenu.open()
                        }
                    }
                    LabelText {
                        Layout.fillWidth: true
                        color: card.record.color || Theme.muted
                        font.pixelSize: 10
                        text: card.record.goal_title || card.record.category_title || 'Unassigned'
                    }
                    LabelText {
                        color: Theme.muted
                        font.pixelSize: 10
                        text: card.record.due_on ? ('Due ' + card.record.due_on.substring(5)) : card.record.recurrence !== 'none' ? '↻ ' + card.record.recurrence : card.record.scheduled_for ? card.record.scheduled_for.substring(5) : ''
                    }
                }
            }
        }
    }
    // Shown on hover or focus over the card's right end, so the card's own layout never moves.
    Rectangle {
        objectName: 'task-quick-delete'
        anchors.right: parent.right; anchors.rightMargin: 6
        anchors.verticalCenter: parent.verticalCenter
        width: deleteButton.implicitWidth; height: deleteButton.implicitHeight
        radius: Theme.cornerRadius
        color: Theme.hover
        visible: card.deletable && !card.selectable && (mouse.containsMouse || deleteButton.hovered || deleteButton.activeFocus)
        SButton {
            id: deleteButton
            anchors.fill: parent
            text: 'Delete'
            quiet: true
            accessibleLabel: 'Delete ' + card.record.title
            onClicked: card.deleteRequested()
        }
    }
    SPopup {
        id: ownerMenu
        objectName: 'task-owner-menu'
        trigger: ownerLabel
        parent: Overlay.overlay
        width: 176
        padding: 6
        contentItem: ColumnLayout {
            spacing: 2
            MenuAction { text: 'Human'; onClicked: { ownerMenu.close(); card.setOwner('human'); } }
            MenuAction { text: 'Agent'; onClicked: { ownerMenu.close(); card.setOwner('agent'); } }
        }
    }
    SPopup {
        id: statusMenu
        trigger: statusButton
        parent: Overlay.overlay
        width: 176
        padding: 6
        contentItem: ColumnLayout {
            spacing: 2
            MenuAction { text: 'To do'; onClicked: { statusMenu.close(); Cal.completion(Backend, card.record, 'todo'); } }
            MenuAction { text: 'In progress'; onClicked: { statusMenu.close(); Cal.completion(Backend, card.record, 'in_progress'); } }
            MenuAction { text: 'Completed'; onClicked: { statusMenu.close(); Cal.completion(Backend, card.record, 'completed'); } }
        }
    }
}
