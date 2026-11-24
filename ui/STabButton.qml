import QtQuick
import QtQuick.Controls

Button {
    id: control
    padding: 0
    property bool active: false

    function moveTab(delta) {
        var siblings = [];
        var children = parent ? parent.children : [];
        for (var i = 0; i < children.length; ++i) {
            var candidate = children[i];
            if (candidate && candidate.enabled && candidate.active !== undefined && candidate.click)
                siblings.push(candidate);
        }
        var index = siblings.indexOf(control);
        if (index < 0 || !siblings.length)
            return;
        var next = (index + delta + siblings.length) % siblings.length;
        siblings[next].forceActiveFocus(Qt.TabFocusReason);
        siblings[next].click();
    }

    onActiveFocusChanged: {
        if (activeFocus && parent && parent.parent && parent.parent.contentX !== undefined) {
            var viewport = parent.parent;
            if (x < viewport.contentX) viewport.contentX = x;
            else if (x + width > viewport.contentX + viewport.width) viewport.contentX = x + width - viewport.width;
        }
    }
    implicitHeight: Theme.tabStripHeight
    implicitWidth: Math.max(Theme.buttonMinWidth, label.implicitWidth + 2 * Theme.controlPadding)
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Text {
        id: label
        text: control.text
        font.family: Theme.font
        font.pixelSize: Theme.fontSizeNormal
        font.weight: Font.Medium
        opacity: control.enabled ? 1 : 0.45
        color: control.active || control.hovered ? Theme.buttonSecondaryText : Theme.muted
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }

    background: Rectangle {
        color: control.hovered && !control.active ? Theme.tabHover : Theme.clear
        radius: Theme.chromeRadius

        Rectangle {
            id: underline
            anchors.bottom: parent.bottom
            anchors.left: parent.left
            anchors.right: parent.right
            height: 2
            color: Theme.accent
            visible: control.active
        }

        Rectangle {
            anchors.fill: parent
            color: Theme.clear
            border.color: Theme.accent
            border.width: 1
            visible: control.visualFocus
            radius: Theme.chromeRadius
        }
    }

    Accessible.role: Accessible.PageTab
    Accessible.name: control.text
    Accessible.selected: control.active

    Keys.onPressed: event => {
        if (!control.enabled) return;
        if (event.key === Qt.Key_Left || event.key === Qt.Key_Up) {
            control.moveTab(-1);
            event.accepted = true;
        } else if (event.key === Qt.Key_Right || event.key === Qt.Key_Down) {
            control.moveTab(1);
            event.accepted = true;
        }
    }
}
