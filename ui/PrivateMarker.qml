import QtQuick
import QtQuick.Controls

// The quiet sign that an item is marked private itself, shown only while
// private mode is off.
SIcon {
    id: marker
    name: 'private'
    size: 12
    color: Theme.muted
    Accessible.role: Accessible.Graphic
    Accessible.name: 'Marked private'
    HoverHandler { id: hover }
    Hint { visible: hover.hovered; delay: 600; text: 'Marked private' }
}
