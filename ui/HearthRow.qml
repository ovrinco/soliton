import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// One row of a Hearth list (a lot, a piece, a constraint): the picture square
// where the list has pictures, the name, then the row's own cells, then on
// hover the three-dot menu. The face opens the item.
Item {
    id: row
    property string name: ''
    property var cells: []          // [{text, mono?, muted?, width?, right?, mark?, markColor?, color?}]
    // The picture file the engine returns; never shown as text. A list with
    // pictures keeps the square on every row so the names line up.
    property string picture: ''
    property bool pictureSlot: false
    property var actions: []        // [{text, run}]
    property bool opens: true
    property int nameWidth: 180
    signal activated()
    implicitHeight: Theme.denseRowHeight

    MouseArea {
        id: face
        anchors.fill: parent
        hoverEnabled: true
        // A row that does not open still shows its menu on hover.
        enabled: row.opens || row.actions.length > 0
        cursorShape: row.opens ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: if (row.opens) row.activated()
    }
    Rectangle { anchors.fill: parent; color: Theme.hover; visible: row.opens && face.containsMouse }
    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 8; anchors.rightMargin: 4
        spacing: row.nameWidth < 180 ? 8 : 16
        Rectangle {
            visible: row.pictureSlot
            implicitWidth: 26; implicitHeight: 26
            color: row.picture ? Theme.raised : 'transparent'
            radius: Theme.cornerRadius
            clip: true
            Image {
                anchors.fill: parent
                visible: !!row.picture
                source: row.picture ? 'file://' + row.picture : ''
                sourceSize: Qt.size(52, 52)
                fillMode: Image.PreserveAspectCrop
                asynchronous: true
                cache: false
            }
        }
        LabelText {
            Layout.preferredWidth: row.nameWidth
            text: row.name
            font.pixelSize: 14
            elide: Text.ElideRight
        }
        Repeater {
            model: row.cells
            // A marked cell carries a dot, in the danger colour unless the cell
            // names its own; its words take the danger colour with it.
            delegate: RowLayout {
                id: cell
                required property var modelData
                Layout.preferredWidth: cell.modelData.width || 120
                Layout.fillWidth: !!cell.modelData.fill
                spacing: 6
                Item { Layout.fillWidth: !!cell.modelData.right; visible: !!cell.modelData.right }
                Rectangle {
                    visible: !!cell.modelData.mark
                    implicitWidth: 6; implicitHeight: 6; radius: 3
                    color: cell.modelData.markColor ? cell.modelData.markColor : Theme.danger
                }
                LabelText {
                    Layout.fillWidth: !cell.modelData.right
                    text: cell.modelData.text
                    font.family: cell.modelData.mono ? Theme.mono : Theme.font
                    font.pixelSize: cell.modelData.small ? 12 : 14
                    color: cell.modelData.color ? cell.modelData.color : cell.modelData.mark && !cell.modelData.markColor ? Theme.danger : cell.modelData.muted ? Theme.muted : Theme.text
                    horizontalAlignment: cell.modelData.right ? Text.AlignRight : Text.AlignLeft
                    elide: Text.ElideRight
                }
            }
        }
        Item { Layout.fillWidth: true }
        Item {
            implicitWidth: 28; implicitHeight: 28
            visible: row.actions.length > 0
            SButton {
                id: moreButton
                anchors.fill: parent
                objectName: 'row-more-' + row.name
                iconName: 'more'; accessibleLabel: 'More actions'; quiet: true
                focusPolicy: Qt.NoFocus
                visible: face.containsMouse || hovered || menu.visible
                onClicked: menu.open()
            }
        }
    }
    SPopup {
        id: menu
        trigger: moreButton
        alignRight: true
        parent: Overlay.overlay
        width: 180
        padding: 6
        contentItem: ColumnLayout {
            spacing: 2
            Repeater {
                model: row.actions
                delegate: MenuAction {
                    required property var modelData
                    objectName: 'row-action-' + row.name + '-' + modelData.text
                    text: modelData.text
                    onClicked: { menu.close(); modelData.run(); }
                }
            }
        }
    }
    Accessible.role: Accessible.ListItem
    Accessible.name: name
}
