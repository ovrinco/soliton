pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts

// The side panel's launcher: the terminal tools installed on this machine.
// A tool that is not installed is absent.
Rectangle {
    id: root
    readonly property var notes: ({
        terminal: 'An ordinary terminal, no agent started',
        codex: 'Codex in a terminal',
        qream: 'Campaign window',
        zeno: 'Executive agent'
    })
    readonly property var toolEntries: Backend.terminalTools.filter(t => Backend.toolsAvailable.indexOf(t) >= 0)
    color: Theme.base

    component Entry: Rectangle {
        id: entry
        required property string tool
        objectName: 'tool-launcher-' + tool
        Layout.fillWidth: true
        implicitHeight: 52
        radius: Theme.cornerRadius
        color: mouse.containsMouse || activeFocus ? Theme.raised : Theme.panel
        border.color: activeFocus ? Theme.accent : Theme.line
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: Backend.moduleNames[tool]
        Keys.onReturnPressed: Backend.chooseTool(tool)
        Keys.onSpacePressed: Backend.chooseTool(tool)
        ColumnLayout {
            anchors.fill: parent; anchors.leftMargin: 14; anchors.rightMargin: 14
            spacing: 2
            LabelText { Layout.fillWidth: true; text: Backend.moduleNames[entry.tool]; font.weight: Font.DemiBold }
            LabelText { Layout.fillWidth: true; text: root.notes[entry.tool]; color: Theme.muted }
        }
        MouseArea { id: mouse; anchors.fill: parent; hoverEnabled: true; onClicked: Backend.chooseTool(entry.tool) }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 20
        spacing: 8

        SectionLabel { text: 'Opens in its own window' }
        Repeater {
            model: root.toolEntries
            delegate: Entry { required property string modelData; tool: modelData }
        }
        LabelText {
            Layout.fillWidth: true
            visible: !root.toolEntries.length
            text: Backend.toolsKnown ? 'No terminal tools are installed.' : 'Looking for installed tools…'
            color: Theme.muted
        }
        Item { Layout.fillHeight: true }
    }
    Component.onCompleted: Backend.refreshTools()
}
