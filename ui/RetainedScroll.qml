import QtQuick

// Presentation coordinates only. Keep each document's key separate from the
// currently observed viewport so delayed saves cannot overwrite its neighbour.
QtObject {
    id: retained
    required property var flickable
    required property string positionKey
    property bool active: flickable.visible
    property real savedX: 0
    property real savedY: 0
    property string observedKey: ''
    property bool ready: false
    function store(key, x, y) {
        if (!key || !Backend.restored || Backend.restoringRoute) return;
        var previous = (Backend.prefs.scrollPositions || {})[key];
        if (previous && previous.x === x && previous.y === y) return;
        var positions = Object.assign({}, Backend.prefs.scrollPositions || {});
        positions[key] = {x:x, y:y};
        Backend.setting('scrollPositions', positions);
    }
    function capture() {
        if (!ready || Backend.restoringRoute || restoreTimer.running) return;
        if (observedKey === positionKey) {
            savedX = flickable.contentX; savedY = flickable.contentY;
            store(observedKey, savedX, savedY);
        }
    }
    function restore() {
        if (active && Backend.restored && positionKey) restoreTimer.restart();
    }
    onPositionKeyChanged: {
        saveTimer.stop();
        if (ready && !restoreTimer.running) store(observedKey, savedX, savedY);
        observedKey = positionKey;
        restore();
    }
    onActiveChanged: {
        if (active) restore();
        else { capture(); saveTimer.stop(); }
    }
    property Timer restoreTimer: Timer {
        interval: 1
        onTriggered: {
            if (retained.active) {
                var position = (Backend.prefs.scrollPositions || {})[retained.positionKey] || {x:0,y:0};
                retained.savedX = Math.max(0, Number(position.x) || 0);
                retained.savedY = Math.max(0, Number(position.y) || 0);
                retained.flickable.contentX = Math.min(retained.savedX, Math.max(0, retained.flickable.contentWidth-retained.flickable.width));
                retained.flickable.contentY = Math.min(retained.savedY, Math.max(0, retained.flickable.contentHeight-retained.flickable.height));
                retained.observedKey = retained.positionKey;
            }
        }
    }
    property Timer saveTimer: Timer { interval:650; onTriggered: if (retained.active) retained.capture() }
    property Connections scrollChanges: Connections {
        target: retained.flickable
        function onContentYChanged() { retained.observe(); }
        function onContentXChanged() { retained.observe(); }
    }
    function observe() {
        if (ready && active && observedKey === positionKey && !Backend.restoringRoute && !restoreTimer.running) {
            savedX = flickable.contentX; savedY = flickable.contentY;
            saveTimer.restart();
        }
    }
    property Connections routeChanges: Connections {
        target: Backend
        function onRouteLeaving() { if (retained.active) retained.capture(); saveTimer.stop(); }
        function onRestoredChanged() { retained.restore(); }
    }
    Component.onCompleted: { observedKey = positionKey; ready = true; restore(); }
}
