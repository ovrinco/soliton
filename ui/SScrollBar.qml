import QtQuick
import QtQuick.Controls
import "Workplane.js" as Workplane

ScrollBar {
    id: control
    objectName: "soliton-scrollbar"
    readonly property bool hasOverflow: Workplane.overflows(size, policy, ScrollBar.AlwaysOff)
    padding: 3
    minimumSize: 0.06
    hoverEnabled: hasOverflow
    interactive: hasOverflow
    implicitWidth: 12
    implicitHeight: 12
    // Retain the attached control's geometry/policy. Only the thumb is painted
    // conditionally: an empty or exact-fit view must not acquire a fake bar.
    contentItem: Rectangle {
        objectName: "soliton-scrollbar-thumb"
        implicitWidth: 6; implicitHeight: 6
        radius: 2
        visible: control.hasOverflow
        color: control.pressed ? Theme.accent : control.hovered ? Theme.text : Theme.muted
        opacity: control.hasOverflow ? (control.active || control.hovered || control.pressed ? 0.9 : 0.28) : 0
    }
    background: Item { }
}
