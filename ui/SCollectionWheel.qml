import QtQuick
import "Workplane.js" as Workplane

// Conventional, immediate mouse-wheel scrolling for ordinary collections.
// Touchpads retain Flickable's native pixel/kinetic handling; modifier gestures
// retain their existing owners. This does not handle calendars, drag or zoom.
WheelHandler {
    id: wheel
    required property var flickable
    target: null
    acceptedDevices: PointerDevice.Mouse
    acceptedModifiers: Qt.NoModifier
    readonly property real lines: Number(Qt.styleHints.wheelScrollLines)
    readonly property real lineHeight: Number(metrics.height)
    // Do not invent a sensitivity when the host does not expose a usable
    // line preference: Flickable then handles the wheel natively.
    readonly property bool usableLines: isFinite(lines) && lines > 0 && isFinite(lineHeight) && lineHeight > 0
    // A handler's event.accepted does not stop delivery; only blocking does.
    // While this handler owns the wheel, Flickable must not also start its own
    // wheel flick, or one notch jumps here and then glides on further.
    // A collection that cannot scroll leaves the wheel to its parents.
    blocking: usableLines && flickable.contentHeight > flickable.height
    enabled: flickable.interactive && !flickable.dragging
    property FontMetrics metrics: FontMetrics { font.family: Theme.font; font.pixelSize: Theme.fontSizeNormal }
    onWheel: event => {
        if (!wheel.blocking) { event.accepted = false; return; }
        var f = wheel.flickable;
        var origin = Number(f.originY) || 0;
        var next = Workplane.wheelPosition(f.contentY, origin, f.contentHeight, f.height,
                    event.pixelDelta.y, event.angleDelta.y, wheel.lines, wheel.lineHeight);
        if (Math.abs(next-f.contentY) < 0.001) { event.accepted = false; return; }
        f.cancelFlick();
        f.contentY = next;
        event.accepted = true;
    }
}
