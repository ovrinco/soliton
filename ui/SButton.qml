import QtQuick
import QtQuick.Controls

Button {
    id: control
    property bool primary: false
    property bool secondary: false
    property bool quiet: false
    property bool destructive: false
    property bool busy: false
    property real restingWidth: 0
    property string iconName: ''
    property string accessibleLabel: text
    // Size and colour come from the theme's one control convention.
    readonly property string colorRole: Theme.buttonRole(primary, destructive, quiet)
    readonly property string interaction: !enabled ? 'disabled' : down ? 'pressed' : hovered ? 'hover' : 'normal'
    readonly property bool square: !text || text.length <= 2
    readonly property color surfaceColor: Theme.buttonFill(colorRole, interaction)
    readonly property color ink: Theme.buttonInk(colorRole, interaction)
    implicitHeight: Theme.controlHeight
    implicitWidth: busy && restingWidth > 0 ? restingWidth
        : square && !(text && iconName) ? Theme.iconButtonSize
        : Math.max(Theme.buttonMinWidth, leftPadding + rightPadding + label.implicitWidth + (iconName ? Theme.chromeIconSize + Theme.controlSpacing : 0))
    padding: 0
    leftPadding: square && !(text && iconName) ? 0 : Theme.controlPadding
    rightPadding: leftPadding
    spacing: Theme.controlSpacing
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    font.weight: primary ? Font.DemiBold : Font.Normal
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    contentItem: Item {
        SIcon { x: control.text ? 0 : (parent.width-width)/2; anchors.verticalCenter: parent.verticalCenter; name: control.iconName; color: control.ink; visible: !!control.iconName && !control.busy }
        Text {
            id: label
            anchors.fill: parent
            anchors.leftMargin: control.iconName ? Theme.chromeIconSize + control.spacing : 0
            visible: !control.busy
            text: control.text; font: control.font; color: control.ink
            horizontalAlignment: Text.AlignHCenter; verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }
        Text {
            anchors.centerIn: parent; visible: control.busy; text: '•••'
            font.family: Theme.font; font.pixelSize: Theme.fontSizeNormal; font.letterSpacing: 3; color: control.ink
            Accessible.ignored: true
        }
    }
    background: Rectangle {
        color: control.surfaceColor
        radius: Theme.chromeRadius
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.accent : Theme.buttonEdge(control.colorRole)
        // Focus on an accent action needs separation, not the same accent edge.
        Rectangle { anchors.fill: parent; anchors.margins: 3; visible: control.visualFocus && control.primary; color: Theme.clear; border.color: control.ink; border.width: 1 }
    }
    ToolTip.visible: !text && !!accessibleLabel && hovered
    ToolTip.text: accessibleLabel
    ToolTip.delay: 600
    Accessible.role: Accessible.Button
    Accessible.name: control.accessibleLabel + (control.busy ? ', working' : '')
    onWidthChanged: if (!busy && width >= Theme.controlHeight) restingWidth = width
}
