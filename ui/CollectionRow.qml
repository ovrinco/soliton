import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "Workplane.js" as Workplane

AbstractButton {
    id: row
    padding: 0
    clip: true
    property string heading: ''
    property string summary: ''
    property string metadata: ''
    property bool selected: false
    property string symbol: 'file'
    property color marker: 'transparent'
    implicitHeight: summary ? 86 : 64
    hoverEnabled: true
    activeFocusOnTab: true
    Accessible.role: Accessible.ListItem
    Accessible.name: heading + (summary ? '. ' + summary : '') + (metadata ? '. ' + metadata : '')
    Accessible.selected: selected
    background: Rectangle {
        color: row.selected ? Theme.wash : row.down ? Theme.selection : row.hovered ? Theme.hover : 'transparent'
        border.width: row.visualFocus ? 2 : 0
        border.color: Theme.accent
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
        Rectangle { width: 2; height: parent.height - 16; y: 8; color: row.marker; visible: row.marker.a > 0 }
    }
    contentItem: Item {
        SIcon { x: 12; y: 18; name: row.symbol; color: row.selected ? Theme.accentText : Theme.muted }
        Column {
            x: 42; y: (parent.height - height) / 2; width: parent.width - 56; spacing: 4
            LabelText { width: parent.width; text: Workplane.previewLine(row.heading); wrapMode: Text.NoWrap; maximumLineCount: 1; elide: Text.ElideRight; font.pixelSize: 15; font.weight: Font.Medium }
            LabelText { width: parent.width; visible: row.summary.length > 0; text: Workplane.previewLine(row.summary); wrapMode: Text.NoWrap; maximumLineCount: 1; elide: Text.ElideRight; color: Theme.muted; font.pixelSize: 13 }
            LabelText { width: parent.width; visible: row.metadata.length > 0; text: Workplane.previewLine(row.metadata); wrapMode: Text.NoWrap; maximumLineCount: 1; elide: Text.ElideRight; color: Theme.muted; font.pixelSize: 12 }
        }
    }
    ToolTip.visible: hovered && (heading.length > 48 || metadata.length > 48 || summary.length > 70)
    ToolTip.text: heading + (metadata ? '\n' + metadata : '') + (summary ? '\n\n' + summary : '')
    ToolTip.delay: 800
}
