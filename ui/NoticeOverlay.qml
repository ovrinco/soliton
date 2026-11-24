import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// Out-of-flow operational feedback. It never changes its parent's layout.
Rectangle {
    id: root
    property string text: ''
    property bool error: false
    signal dismissed()
    z: 1000
    visible: text.length > 0
    width: Math.min(520, parent ? parent.width - 24 : 520)
    implicitHeight: Math.min(280, noticeText.implicitHeight + 24)
    height: implicitHeight
    color: Theme.panel
    border.color: error ? Theme.dangerText : Theme.edge
    Accessible.role: Accessible.AlertMessage
    Accessible.name: text
    Rectangle { width: 3; height: parent.height; color: root.error ? Theme.dangerText : Theme.accent }
    RowLayout {
        id: noticeRow; anchors.fill: parent; anchors.margins: 12; spacing: 12
        Flickable {
            Layout.fillWidth: true; Layout.fillHeight: true; clip: true
            contentWidth: width; contentHeight: noticeText.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar { }
            TextEdit { id: noticeText; width: parent.width; text: root.text; readOnly: true; selectByMouse: true; wrapMode: TextEdit.Wrap; textFormat: TextEdit.PlainText; color: root.error ? Theme.dangerText : Theme.text; font.family: Theme.font; font.pixelSize: 13 }
        }
        SButton { text: ''; iconName: 'close'; quiet: true; accessibleLabel: 'Dismiss message'; onClicked: root.dismissed() }
    }
}
