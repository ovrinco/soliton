import QtQuick

// Holds the room of a transient control whether or not it is showing, so
// revealing it moves nothing. The child is the control; its own visible
// property shows or hides it inside the held room.
Item {
    id: slot
    default property Item control
    implicitWidth: control ? control.implicitWidth : 0
    implicitHeight: control ? control.implicitHeight : 0
    onControlChanged: {
        if (!control) return;
        control.parent = slot;
        control.anchors.centerIn = slot;
    }
}
