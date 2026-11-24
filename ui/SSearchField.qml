import QtQuick
import QtQuick.Controls

Field {
    id: control
    // Every caller names what it searches; there is no generic fallback hint.
    required property string scopeName
    // The hint names what is searched, from the same scope as the accessible name.
    // Where the field is too narrow for the whole hint, the subject alone is shown,
    // so elision never leaves only the verb.
    TextMetrics { id: fullHint; font: control.font; text: "Search " + control.scopeName }
    placeholderText: fullHint.advanceWidth <= control.width - control.leftPadding - control.rightPadding
        ? fullHint.text : scopeName.charAt(0).toUpperCase() + scopeName.slice(1)
    Accessible.name: "Search " + scopeName
    // No search icon inside the field: the header carries the one search icon.
    leftPadding: Theme.controlPadding
    rightPadding: Theme.innerButtonSize + Theme.space4
    SButton {
        objectName: "search-clear"
        anchors.right: parent.right; anchors.verticalCenter: parent.verticalCenter
        anchors.rightMargin: Theme.space4 / 2
        width: Theme.innerButtonSize; height: Theme.innerButtonSize; implicitWidth: Theme.innerButtonSize; implicitHeight: Theme.innerButtonSize
        iconName: "close"; quiet: true; accessibleLabel: "Clear search"
        visible: control.text.length > 0
        onClicked: { control.clear(); control.textEdited(); control.forceActiveFocus(); }
    }
}
