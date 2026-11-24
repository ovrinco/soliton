import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// A section in the same document, never another task tab. Explicit expansion
// alone changes its height; saving/selection/background replies do not.
ColumnLayout {
    id: root
    property string title: ''
    property string summary: ''
    property bool expanded: false
    default property alias body: bodyColumn.data
    spacing: 8
    AbstractButton {
        id: trigger
        Layout.fillWidth: true
        implicitHeight: 40
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        onClicked: root.expanded = !root.expanded
        Accessible.name: root.title + (root.summary ? ', ' + root.summary : '')
        Accessible.role: Accessible.Button
        Accessible.description: root.expanded ? 'Expanded' : 'Collapsed'
        contentItem: RowLayout {
            spacing: 8
            LabelText { text: root.title; font.weight: Font.Medium }
            LabelText { Layout.fillWidth: true; text: root.summary; color: Theme.muted; horizontalAlignment: Text.AlignRight; font.pixelSize: 12 }
            SIcon { name: root.expanded ? 'down' : 'right'; color: Theme.muted; implicitWidth: 16; implicitHeight: 16 }
        }
        background: Rectangle {
            color: trigger.hovered ? Theme.hover : 'transparent'
            border.color: trigger.visualFocus ? Theme.accent : 'transparent'
            border.width: 2
            Rectangle { width: parent.width; height: 1; color: Theme.line }
        }
    }
    ColumnLayout {
        id: bodyColumn
        Layout.fillWidth: true
        visible: root.expanded
        spacing: 10
    }
}
