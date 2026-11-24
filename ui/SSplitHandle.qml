import QtQuick
import QtQuick.Controls

// The hit region is not the visual separator. Deliberate resizing changes the
// neighboring panes only; this component never animates or adds a layout row.
// A double click recentres: the pane before this handle is sized so the
// handle meets the middle of the view (the first pane takes half of it).
Rectangle {
    id: handle
    property bool horizontal: true
    // Keep the pointer target close to the pane boundary. The visible line is
    // centered inside this six-pixel target, so there is no adjacent dead gap.
    implicitWidth: horizontal ? 6 : 1
    implicitHeight: horizontal ? 1 : 10
    color: "transparent"

    function splitView() {
        for (var item = handle.parent; item; item = item.parent)
            if (item instanceof SplitView) return item;
        return null;
    }
    function recentre() {
        var view = splitView();
        if (!view) return;
        var vertical = view.orientation === Qt.Vertical;
        var start = vertical ? handle.y : handle.x;
        var pane = null;
        for (var i = 0; i < view.contentChildren.length; ++i) {
            var candidate = view.contentChildren[i];
            if (!candidate.visible) continue;
            var end = vertical ? candidate.y + candidate.height : candidate.x + candidate.width;
            if (Math.abs(end - start) < 1) pane = candidate;
        }
        if (!pane) return;
        var size = Math.round((vertical ? view.height : view.width) / 2 - (vertical ? pane.y : pane.x));
        if (size <= 0) return;
        if (vertical) pane.SplitView.preferredHeight = size;
        else pane.SplitView.preferredWidth = size;
    }

    Rectangle {
        anchors.centerIn: parent
        width: handle.horizontal ? 1 : parent.width
        height: handle.horizontal ? parent.height : 1
        color: handle.SplitHandle.hovered || handle.SplitHandle.pressed ? Theme.accent : Theme.line
        visible: true
    }
    HoverHandler { cursorShape: handle.horizontal ? Qt.SplitHCursor : Qt.SplitVCursor }
    // The split view owns the handle's pointer events, so a double click is
    // read from its pressed state: a second press within the platform's
    // double-click time, the handle not moved, recentres on release.
    property real lastPress: 0
    property real lastPosition: -1
    property bool recentrePending: false
    SplitHandle.onPressedChanged: {
        var position = horizontal ? x : y;
        if (!SplitHandle.pressed) {
            if (recentrePending) recentre();
            recentrePending = false;
            return;
        }
        var now = Date.now();
        recentrePending = now - lastPress < Application.styleHints.mouseDoubleClickInterval && Math.abs(position - lastPosition) < 1;
        lastPress = recentrePending ? 0 : now;
        lastPosition = position;
    }
}
