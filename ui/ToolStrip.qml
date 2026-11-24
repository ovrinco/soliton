import QtQuick
import QtQuick.Layouts

// The top of the right slot: the control that pops the tool out.
Rectangle {
    id: strip
    property string tool: ''
    anchors.left: parent.left; anchors.right: parent.right; anchors.top: parent.top
    // Only while a pop-out is possible; with one tool out, the slot is the tool alone.
    height: Backend.poppedTool === '' ? 32 : 0
    visible: height > 0
    color: Theme.panel
    RowLayout {
        anchors.fill: parent; anchors.rightMargin: 8; spacing: 0
        Item { Layout.fillWidth: true }
        SButton {
            objectName: 'tool-pop-out-' + strip.tool
            quiet: true
            text: 'Pop out'
            onClicked: Backend.popOutTool()
        }
    }
    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
}
