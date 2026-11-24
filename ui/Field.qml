import QtQuick
import QtQuick.Controls

TextField {
    id: field
    property bool invalid: false
    property bool frameless: false
    property string errorText: ''
    implicitHeight: Theme.controlHeight
    implicitWidth: Theme.inputWidth
    leftPadding: Theme.controlPadding
    rightPadding: Theme.controlPadding
    topPadding: 0
    bottomPadding: 0
    verticalAlignment: TextInput.AlignVCenter
    color: field.enabled ? Theme.inputText : Theme.buttonDisabledText
    placeholderTextColor: Theme.inputPlaceholder
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    selectByMouse: true
    selectionColor: Theme.accent
    selectedTextColor: Theme.accentForeground
    background: Rectangle {
        color: field.frameless ? Theme.clear : field.enabled ? Theme.inputFill : Theme.buttonDisabledFill
        border.width: field.activeFocus || field.invalid ? 1 : 0
        border.color: field.invalid ? Theme.dangerText : Theme.accent
        Rectangle {
            anchors.bottom: parent.bottom
            width: parent.width; height: 1
            visible: !field.frameless && !field.activeFocus && !field.invalid
            color: Theme.fieldLine
        }
    }
    Accessible.name: placeholderText
    Accessible.description: errorText
    Accessible.role: Accessible.EditableText
}
