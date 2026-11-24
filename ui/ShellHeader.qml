import QtQuick
import QtQuick.Controls
import QtQuick.Effects
import QtQuick.Layouts

// The header: mark, page name, the search or command entry, the privacy
// control. Heights follow Theme.
Rectangle {
    id: header
    signal paletteRequested(string seed)
    readonly property string pageName: Backend.page === 'Goals' ? 'Categories & goals' : Backend.page
    color: Theme.panel
    objectName: 'soliton-header'
    implicitHeight: Theme.shellHeight

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: 12
        anchors.rightMargin: 8
        spacing: 10

        MouseArea {
            Layout.preferredHeight: 24
            Layout.preferredWidth: 24
            cursorShape: Qt.PointingHandCursor
            activeFocusOnTab: true
            Accessible.role: Accessible.Button
            Accessible.name: 'Soliton'
            onClicked: Backend.navigate('Dashboard')
            Keys.onReturnPressed: Backend.navigate('Dashboard')
            Keys.onSpacePressed: Backend.navigate('Dashboard')

            Image {
                anchors.fill: parent
                fillMode: Image.PreserveAspectFit
                layer.enabled: true
                mipmap: true
                source: '../assets/soliton-monochrome.png'
                sourceSize: Qt.size(64, 64)
                layer.effect: MultiEffect {
                    colorization: 1
                    colorizationColor: Theme.text
                }
            }
        }
        // The small divider between the mark and the page name.
        Rectangle {
            objectName: 'soliton-header-divider'
            Layout.alignment: Qt.AlignVCenter
            implicitWidth: 1; implicitHeight: 16
            color: Theme.line
        }
        LabelText {
            objectName: 'soliton-page-title'
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignLeft | Qt.AlignVCenter
            Layout.maximumWidth: Theme.commandBar ? Math.max(0, (header.width - commandBar.width) / 2 - 56) : -1
            Layout.minimumWidth: 0
            text: header.pageName
            font.pixelSize: 14; font.weight: Font.Medium
            elide: Text.ElideRight
        }
        Item { Layout.fillWidth: true; visible: Theme.commandBar }
        SButton {
            visible: !Theme.commandBar
            quiet: true
            iconName: 'search'
            accessibleLabel: 'Search'
            text: ''
            onClicked: header.paletteRequested('')
        }
        // The privacy control: one quiet toggle that never moves or hides.
        MouseArea {
            id: privateControl
            objectName: 'soliton-private'
            Layout.preferredHeight: Theme.shellHeight - 4
            Layout.preferredWidth: privateRow.implicitWidth + 12
            cursorShape: Qt.PointingHandCursor
            activeFocusOnTab: true
            Accessible.role: Accessible.CheckBox
            Accessible.name: 'Private'
            Accessible.checkable: true
            Accessible.checked: Backend.privateMode
            onClicked: Backend.togglePrivateMode()
            Keys.onReturnPressed: Backend.togglePrivateMode()
            Keys.onSpacePressed: Backend.togglePrivateMode()
            // The icon holds its place at the right; the word grows leftward.
            Row {
                id: privateRow
                anchors.right: parent.right; anchors.rightMargin: 6
                anchors.verticalCenter: parent.verticalCenter
                layoutDirection: Qt.RightToLeft
                spacing: 6
                SIcon {
                    anchors.verticalCenter: parent.verticalCenter
                    name: 'private'; size: 16
                    color: Backend.privateMode ? Theme.accent : Theme.muted
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: Backend.privateMode
                    text: 'Private'
                    font.family: Theme.font; font.pixelSize: 13
                    color: Theme.accent
                }
            }
        }
    }
    Rectangle {
        id: commandBar
        anchors.centerIn: parent
        visible: Theme.commandBar
        width: Math.min(Theme.commandBarWidth, Math.max(0, parent.width - 240))
        height: 28
        radius: Theme.chromeRadius
        color: Theme.fieldSurface
        border.width: 1
        border.color: barInput.activeFocus ? Theme.accent : Theme.line
        SIcon { x: 8; anchors.verticalCenter: parent.verticalCenter; name: 'search'; color: Theme.muted }
        TextInput {
            id: barInput
            anchors.fill: parent; anchors.leftMargin: 32; anchors.rightMargin: 8
            verticalAlignment: TextInput.AlignVCenter
            font.family: Theme.font; font.pixelSize: 13
            color: Theme.text; selectionColor: Theme.selection
            clip: true
            Accessible.name: 'Search or run a command'
            onTextEdited: if (text) { var seed = text; text = ''; header.paletteRequested(seed); }
            Keys.onReturnPressed: header.paletteRequested('')
            Text {
                anchors.fill: parent
                verticalAlignment: Text.AlignVCenter
                visible: !barInput.text
                text: 'Search or run a command'
                font: barInput.font; color: Theme.muted
                elide: Text.ElideRight
            }
        }
        MouseArea { anchors.fill: parent; cursorShape: Qt.IBeamCursor; onClicked: header.paletteRequested('') }
    }
}
