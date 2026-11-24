import QtQuick
import QtQuick.Controls

// One rail item. The icon keeps its x whether the rail is folded or not; the
// active item sits on the raised surface with its icon in the accent.
AbstractButton {
    id: control
    padding: 0
    property bool selected: false
    property bool collapsed: false
    property bool showText: true
    property string symbol: ''
    property int iconX: 11
    implicitWidth: 168
    implicitHeight: 40
    checked: selected
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    contentItem: Item {
        SIcon {
            objectName: 'navigation-icon'
            x: control.iconX
            anchors.verticalCenter: parent.verticalCenter
            size: Theme.railIconSize
            name: control.symbol
            color: control.selected ? Theme.accentText : control.hovered ? Theme.text : Theme.muted
        }
        Text {
            x: control.iconX + Theme.railIconSize + 10; width: Math.max(0,parent.width-x-8); height: parent.height
            visible: !control.collapsed && control.showText && !!control.text
            text: control.text
            font.family: Theme.font; font.pixelSize: Theme.railTextSize
            font.weight: control.selected ? Font.Medium : Font.Normal
            color: control.selected || control.hovered ? Theme.text : Theme.muted
            verticalAlignment: Text.AlignVCenter
            elide: Text.ElideRight
        }
    }
    background: Rectangle {
        radius: Theme.chromeRadius
        color: control.selected ? Theme.raised : control.hovered ? Theme.hover : 'transparent'
        Rectangle { anchors.fill: parent; radius: Theme.chromeRadius; color: 'transparent'; border.width: 2; border.color: Theme.accent; visible: control.visualFocus }
    }
    onActiveFocusChanged: {
        if (!activeFocus) return;
        for (var viewport = parent; viewport; viewport = viewport.parent) {
            if (viewport.contentY !== undefined && viewport.contentItem) {
                var point = mapToItem(viewport.contentItem, 0, 0);
                if (point.y < viewport.contentY) viewport.contentY = point.y;
                else if (point.y + height > viewport.contentY + viewport.height)
                    viewport.contentY = point.y + height - viewport.height;
                break;
            }
        }
    }
    ToolTip.visible: (collapsed || !showText) && hovered && !!text
    ToolTip.text: text
    ToolTip.delay: 600
    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: selected
}
