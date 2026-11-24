import QtQuick
import QtQuick.Controls

Slider {
    id: control
    implicitWidth: horizontal ? Theme.inputWidth : Theme.controlHeight
    implicitHeight: horizontal ? Theme.controlHeight : Theme.inputWidth
    hoverEnabled: true
    background: Rectangle {
        x: control.leftPadding + (control.horizontal ? 0 : (control.availableWidth - width)/2)
        y: control.topPadding + (control.horizontal ? (control.availableHeight-height)/2 : 0)
        implicitWidth: 120; implicitHeight: 3
        width: control.horizontal ? control.availableWidth : 3; height: control.horizontal ? 3 : control.availableHeight; color: Theme.edge
        Rectangle { x: control.mirrored && control.horizontal ? parent.width-width : 0; y: control.horizontal ? 0 : parent.height-height; width: control.horizontal ? control.position*parent.width : parent.width; height: control.horizontal ? parent.height : control.position*parent.height; color: control.enabled ? Theme.accent : Theme.muted }
    }
    handle: Rectangle {
        x: control.leftPadding + (control.horizontal ? control.visualPosition * (control.availableWidth - width) : (control.availableWidth-width)/2)
        y: control.topPadding + (control.horizontal ? (control.availableHeight-height)/2 : control.visualPosition*(control.availableHeight-height))
        width: Theme.sliderHandleSize; height: Theme.sliderHandleSize
        color: control.pressed ? Theme.selection : Theme.panel
        border.color: !control.enabled ? Theme.line : control.visualFocus ? Theme.accent : Theme.muted
        border.width: control.visualFocus ? 2 : 1
        Rectangle { anchors.centerIn: parent; width: 4; height: 8; color: control.enabled ? Theme.accent : Theme.muted }
    }
}
