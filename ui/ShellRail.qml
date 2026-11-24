import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The left rail: page items, Settings, and the fold chevron at the bottom.
// Folded or not, every icon stays at the same x.
Rectangle {
    id: rail
    readonly property bool collapsed: Theme.railText && Backend.navCollapsed
    readonly property int railWidth: collapsed ? Theme.railFoldedWidth : Theme.railWidth
    readonly property int iconX: Math.round((Theme.railFoldedWidth - Theme.railIconSize) / 2)
    signal foldRequested()
    objectName: 'soliton-navigation'
    color: Theme.panel
    implicitWidth: railWidth

    component RailItem: SNavItem {
        width: parent ? parent.width : 0
        implicitHeight: Theme.railItemHeight
        iconX: rail.iconX
        collapsed: rail.collapsed
        showText: Theme.railText
    }

    ColumnLayout {
        anchors.fill: parent; anchors.topMargin: 6; anchors.bottomMargin: 6; spacing: 2
        Flickable {
            id: navigationScroll
            Layout.fillWidth: true; Layout.fillHeight: true; Layout.minimumHeight: 0
            clip: true; contentWidth: width; contentHeight: navigationItems.implicitHeight
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: SScrollBar { }
            Column {
                id: navigationItems
                width: navigationScroll.width; spacing: 2
                Repeater {
                    model: ['Dashboard','Learning','Strategy']
                    delegate: RailItem {
                        required property string modelData
                        objectName: 'navigation-' + modelData
                        selected: Backend.page === modelData
                        symbol: modelData.toLowerCase(); text: modelData
                        onClicked: Backend.navigate(modelData)
                    }
                }
                // The one partition of the left panel, beneath the first three.
                Item {
                    width: parent.width; height: 10
                    Rectangle {
                        objectName: 'navigation-partition'
                        anchors.centerIn: parent
                        width: parent.width - 16; height: 1
                        color: Theme.line
                    }
                }
                Repeater {
                    model: ['Goals','Tasks','Hearth','Notes','Portal']
                    delegate: RailItem {
                        required property string modelData
                        objectName: 'navigation-' + modelData
                        selected: Backend.page === modelData
                        symbol: modelData.toLowerCase()
                        text: modelData
                        onClicked: Backend.navigate(modelData)
                    }
                }
            }
        }
        RailItem {
            objectName: 'navigation-Settings'
            Layout.fillWidth: true
            text: 'Settings'; symbol: 'settings'; selected: Backend.page === 'Settings'
            onClicked: Backend.navigate('Settings')
        }
        RailItem {
            objectName: 'navigation-toggle'
            Layout.fillWidth: true
            // The chevron points the way the fold will move.
            symbol: Theme.treatment === 2 ? (Backend.navCollapsed ? 'right' : 'left')
                : (rail.collapsed ? 'right' : 'left')
            text: ''
            Accessible.role: Accessible.Button
            Accessible.name: (Theme.treatment === 2 ? Backend.navCollapsed : rail.collapsed) ? 'Expand navigation' : 'Collapse navigation'
            onClicked: rail.foldRequested()
        }
    }
}
