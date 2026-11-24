import QtQuick
import QtQuick.Layouts

Item {
    id: bar
    implicitHeight: Theme.tabStripHeight
    implicitWidth: layout.implicitWidth
    default property alias content: layout.data
    property alias spacing: layout.spacing
    Flickable {
        id: viewport
        anchors.fill: parent
        contentWidth: layout.width; contentHeight: height
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        flickableDirection: Flickable.HorizontalFlick
        RowLayout {
            id: layout
            width: Math.max(viewport.width, implicitWidth)
            height: viewport.height
            spacing: Theme.space4
        }
    }
    Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line; z: -1 }
    Accessible.role: Accessible.PageTabList
}
