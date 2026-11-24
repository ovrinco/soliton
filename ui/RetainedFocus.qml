import QtQuick
import QtQuick.Window

// Transient focus pointers, never persisted as model state. Invalidated QObject
// pointers become null. Native popup focus restoration remains Qt's job.
QtObject {
    id: memory
    required property Item scope
    property string positionKey: 'default'
    property bool active: scope.visible
    property var targets: ({})
    property bool ready: false
    function contains(item) {
        for (var current = item; current; current = current.parent)
            if (current === scope) return true;
        return false;
    }
    function remember() {
        var window = scope.Window.window;
        var item = window ? window.activeFocusItem : null;
        if (active && contains(item)) {
            var next = Object.assign({}, targets); next[positionKey] = item; targets = next;
        }
    }
    function restore() {
        if (!ready || !active) return;
        var key = positionKey;
        Qt.callLater(function() {
            var item = targets[key];
            if (memory.active && memory.positionKey === key && item && item.visible && item.enabled && contains(item))
                item.forceActiveFocus(Qt.OtherFocusReason);
        });
    }
    onActiveChanged: if (active) restore()
    onPositionKeyChanged: restore()
    property Connections windowChanges: Connections {
        target: memory.scope.Window.window
        function onActiveFocusItemChanged() { memory.remember(); }
    }
    Component.onCompleted: ready = true
}
