import QtQuick

// Hidden routes retain their delegates while another route restores its filters.
QtObject {
    id: retained
    required property var sourceModel
    required property bool active
    property var model: []
    function reconcile() {
        if (!active || Backend.restoringRoute) return;
        var next = sourceModel || [];
        if (model.length === next.length && model.every((entry, i) => entry === next[i])) return;
        model = next;
    }
    onSourceModelChanged: reconcile()
    onActiveChanged: reconcile()
    property Connections navigation: Connections {
        target: Backend
        function onRestoringRouteChanged() { retained.reconcile(); }
    }
    Component.onCompleted: reconcile()
}
