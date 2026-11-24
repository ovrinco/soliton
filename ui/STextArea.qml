import QtQuick
import QtQuick.Controls

TextArea {
    id: control
    property bool invalid: false
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    color: Theme.text
    placeholderTextColor: Theme.muted
    selectionColor: Theme.accent
    selectedTextColor: Theme.accentForeground
    selectByMouse: true
    selectByKeyboard: true
    padding: 10
    background: Rectangle {
        color: Theme.fieldSurface
        border.width: control.activeFocus || control.invalid ? 1 : 0
        border.color: control.invalid ? Theme.dangerText : Theme.accent
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: control.invalid ? Theme.dangerText : control.activeFocus ? Theme.accent : Theme.fieldLine }
        radius: Theme.chromeRadius
    }
}
