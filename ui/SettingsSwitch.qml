import QtQuick
import QtQuick.Controls

AbstractButton {
    id: control
    implicitWidth: 34; implicitHeight: 20
    focusPolicy: Qt.NoFocus
    Accessible.role: Accessible.CheckBox
    Accessible.checked: checked
    indicator: Rectangle {
        width: 34; height: 20; radius: 10
        anchors.verticalCenter: parent.verticalCenter
        color: control.checked ? Theme.accent : 'transparent'
        border.color: control.checked ? Theme.accent : Theme.muted
        opacity: control.enabled ? 1 : .5
        Rectangle {
            width: 14; height: 14; radius: 7
            anchors.verticalCenter: parent.verticalCenter
            x: control.checked ? parent.width - width - 3 : 3
            color: control.checked ? Theme.accentForeground : Theme.muted
            Behavior on x { enabled: !Theme.reducedMotion; NumberAnimation { duration: 90 } }
        }
    }
    contentItem: Item {}
    background: Item {}
}
