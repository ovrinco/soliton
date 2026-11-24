import QtQuick
import QtQuick.Controls

Popup {
    id: control
    // Menus own keyboard dismissal while open, including nested menus whose
    // callers do not set focus explicitly. Qt restores the previous focus.
    focus: true
    // Set this on anchored menus. Coordinates are recalculated in the overlay
    // so pane and window resizes keep the menu attached to its own control.
    property Item trigger: null
    property bool alignRight: false
    property int anchorGap: 4

    // Private mode turning on closes every open popup, so none stays open on
    // an item that has just left the screen.
    Connections {
        target: Backend
        function onPrivateModeChanged() { if (Backend.privateMode && control.visible) control.close(); }
    }

    function positionAtTrigger() {
        if (!trigger || !parent)
            return;
        var point = trigger.mapToItem(parent, alignRight ? trigger.width : 0, trigger.height + anchorGap);
        var margin = 8;
        var nextX = alignRight ? point.x - width : point.x;
        if (nextX + width > parent.width - margin)
            nextX = parent.width - width - margin;
        x = Math.max(margin, nextX);
        y = point.y;
        if (y + height > parent.height - margin)
            y = trigger.mapToItem(parent, 0, -height - anchorGap).y;
        y = Math.max(margin, Math.min(y, parent.height - height - margin));
    }

    // The privacy key reaches past an open menu or palette: the window's own
    // shortcut is blocked while a popup holds focus.
    Shortcut {
        sequence: 'Ctrl+Shift+P'
        enabled: control.visible
        onActivated: Backend.togglePrivateMode()
        onActivatedAmbiguously: Backend.togglePrivateMode()
    }

    onOpened: positionAtTrigger()
    onTriggerChanged: if (visible) positionAtTrigger()
    onWidthChanged: if (visible) positionAtTrigger()
    onHeightChanged: if (visible) positionAtTrigger()

    Connections {
        target: control.trigger
        function onXChanged() { control.positionAtTrigger(); }
        function onYChanged() { control.positionAtTrigger(); }
        function onWidthChanged() { control.positionAtTrigger(); }
        function onHeightChanged() { control.positionAtTrigger(); }
    }

    Connections {
        target: control.parent
        function onWidthChanged() { if (control.visible) control.positionAtTrigger(); }
        function onHeightChanged() { if (control.visible) control.positionAtTrigger(); }
    }
    // Ancestors can move without changing trigger.x/y (for example a split pane).
    Timer {
        interval: 16
        running: control.visible && !!control.trigger
        repeat: true
        onTriggered: control.positionAtTrigger()
    }
    palette.base: Theme.panel
    palette.text: Theme.text
    palette.window: Theme.base
    palette.windowText: Theme.text
    palette.button: Theme.raised
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.accentForeground
    font.family: Theme.font
    font.pixelSize: Theme.fontSizeNormal
    background: Rectangle { color: Theme.panel; border.color: Theme.edge }
}
