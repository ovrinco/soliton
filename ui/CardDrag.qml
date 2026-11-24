import QtQuick
import QtQuick.Controls

// A single drag geometry for every card. The pointer remains at its original
// position inside a full-size snapshot, including when dragging across panes.
MouseArea {
    id: surface
    objectName: "card-drag-surface"

    required property Item card
    required property var record
    property var keys: ["task"]
    property point grabPoint: Qt.point(0, 0)
    property bool draggable: true
    property bool dragging: false
    property bool moved: false
    property bool dropHandled: false
    property bool cancelled: false
    property alias dragPreview: ghost
    signal activated
    signal opened

    anchors.fill: parent
    cursorShape: dragging ? Qt.ClosedHandCursor : draggable ? Qt.OpenHandCursor : Qt.PointingHandCursor
    hoverEnabled: true
    preventStealing: true

    Component.onDestruction: if (Backend.draggingCard === surface)
        Backend.draggingCard = null
    onCanceled: {
        ghost.Drag.cancel();
        dragging = false;
        dropHandled = false;
        ghost.x = -10000;
    }
    onPositionChanged: mouse => {
        if (!pressed || !draggable || cancelled)
            return;
        var local = mapToItem(card, mouse.x, mouse.y);
        if (moved || Math.abs(local.x - grabPoint.x) + Math.abs(local.y - grabPoint.y) >= 7) {
            moved = true;
            if (surface.Window.window && surface.Window.window.contentItem && ghost.parent !== surface.Window.window.contentItem)
                ghost.parent = surface.Window.window.contentItem;
            var p = mapToGlobal(mouse.x, mouse.y);
            var overlayP = ghost.parent.mapFromGlobal(p.x, p.y);
            ghost.x = overlayP.x - grabPoint.x;
            ghost.y = overlayP.y - grabPoint.y;
            dragging = true;
        }
    }
    onClicked: if (!moved)
        activated()
    onDoubleClicked: if (!moved)
        opened()
    onDraggingChanged: {
        if (dragging)
            Backend.draggingCard = surface;
        else if (Backend.draggingCard === surface)
            Backend.draggingCard = null;
    }
    onPressed: mouse => {
        forceActiveFocus();
        moved = false;
        dropHandled = false;
        cancelled = false;
        if (surface.Window.window && surface.Window.window.contentItem && ghost.parent !== surface.Window.window.contentItem)
            ghost.parent = surface.Window.window.contentItem;
        grabPoint = mapToItem(card, mouse.x, mouse.y);
        var p = card.mapToGlobal(0, 0);
        var overlayP = ghost.parent.mapFromGlobal(p.x, p.y);
        ghost.x = overlayP.x;
        ghost.y = overlayP.y;
        preview.scheduleUpdate();
    }
    onReleased: {
        if (dragging)
            ghost.Drag.drop();
        dragging = false;
        ghost.x = -10000;
        ghost.y = -10000;
    }

    Keys.onEscapePressed: event => {
        if (dragging) {
            cancelled = true;
            ghost.Drag.cancel();
            dragging = false;
            event.accepted = true;
        } else event.accepted = false;
    }

    Item {
        id: ghost

        Drag.hotSpot: surface.grabPoint
        Drag.keys: surface.keys
        Drag.active: surface.dragging
        Drag.source: surface
        height: surface.card.height
        objectName: 'card-drag-preview'
        opacity: .94
        parent: (surface.Window.window && surface.Window.window.contentItem) ? surface.Window.window.contentItem : (Overlay.overlay || surface)
        visible: surface.dragging
        width: surface.card.width
        x: -10000
        z: 99999

        ShaderEffectSource {
            id: preview

            anchors.fill: parent
            hideSource: false
            live: false
            recursive: false
            sourceItem: surface.card
        }
        Rectangle {
            anchors.fill: parent
            border.color: Theme.accent
            border.width: 2
            color: 'transparent'
            radius: Theme.cornerRadius
        }
    }
}
