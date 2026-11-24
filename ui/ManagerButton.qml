import QtQuick
import QtQuick.Controls

// The bordered action of the skill and plugin managers, in Soliton's tokens:
// quiet with a hairline; selected for the current view; primary is the one
// filled action of a surface; danger is red text.
Button {
    id: control
    property bool selected: false
    property bool primary: false
    property bool danger: false
    property bool leftAlign: false
    readonly property bool filled: primary && enabled
    readonly property color ink: !enabled ? Theme.muted
        : primary ? Theme.accentForeground
        : danger ? Theme.dangerText
        : selected ? Theme.text : Theme.text
    implicitHeight: 32
    implicitWidth: Math.max(32, label.implicitWidth + 26)
    padding: 0
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    font.family: Theme.mono
    font.pixelSize: 13
    font.weight: selected || primary ? Font.DemiBold : Font.Normal
    contentItem: Text {
        id: label
        text: control.text
        font: control.font
        color: control.ink
        leftPadding: control.leftAlign ? 12 : 0
        horizontalAlignment: control.leftAlign ? Text.AlignLeft : Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: Theme.chromeRadius
        color: control.filled ? (control.down ? Theme.mixed(Theme.accent, Theme.accentForeground, .16) : control.hovered ? Theme.mixed(Theme.accent, Theme.accentForeground, .08) : Theme.accent)
            : control.down ? Theme.selection
            : control.selected ? Theme.raised
            : control.hovered ? Theme.hover : 'transparent'
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.accent : control.filled ? 'transparent'
            : control.selected ? Theme.edge : Theme.line
    }
    Accessible.role: Accessible.Button
    Accessible.name: text
}
