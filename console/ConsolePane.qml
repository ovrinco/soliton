import QtQuick
import QtQuick.Layouts
import "../ui" as Ui

// The console module: a terminal tool chosen from the launcher. The tool runs in
// its own window from the terminal emulator; this card names it, says where it
// started and closes without touching that window.
Rectangle {
    id: root
    property string tool: 'terminal'
    signal hideRequested
    readonly property var launch: Ui.Backend.toolLaunches[tool] || null
    readonly property string folderName: launch && launch.folder ? launch.folder.split('/').filter(p => !!p).pop() || '' : ''
    readonly property var notes: ({
        terminal: 'A terminal in the folder of the view you are on, with no agent started.',
        codex: 'Codex in a terminal, in the folder of the view you are on.',
        qream: 'QREAM opens its campaign window.',
        zeno: 'Zeno opens in the folder of the view you are on.'
    })
    color: Ui.Theme.base

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 24
        spacing: 12

        Ui.LabelText {
            objectName: 'tool-module-name'
            Layout.fillWidth: true
            text: Ui.Backend.moduleNames[root.tool] || ''
            font.pixelSize: 18
            font.weight: Font.DemiBold
        }
        Ui.LabelText {
            Layout.fillWidth: true
            text: root.notes[root.tool] || ''
            color: Ui.Theme.muted
            wrapMode: Text.Wrap
            elide: Text.ElideNone
        }
        Ui.LabelText {
            objectName: 'tool-module-state'
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            elide: Text.ElideNone
            text: !root.launch ? 'Not opened from this window yet.'
                : !root.launch.started ? 'It did not open.'
                : 'Open in its own window.'
        }
        Ui.LabelText {
            objectName: 'tool-module-folder'
            Layout.fillWidth: true
            visible: root.tool !== 'qream' && !!root.folderName && !!root.launch && root.launch.started
            text: 'Folder: ' + root.folderName
        }
        RowLayout {
            spacing: 8
            Ui.SButton {
                objectName: 'tool-module-open'
                primary: !root.launch
                text: root.launch && root.launch.started ? 'Open again' : 'Open'
                enabled: Ui.Backend.toolsAvailable.indexOf(root.tool) >= 0
                onClicked: Ui.Backend.launchTool(root.tool)
            }
            Ui.SButton {
                objectName: 'tool-module-close'
                text: 'Close'
                onClicked: root.hideRequested()
            }
        }
        Ui.LabelText {
            Layout.fillWidth: true
            visible: !!root.launch && root.launch.started
            text: 'Close leaves the window open.'
            color: Ui.Theme.muted
        }
        Item { Layout.fillHeight: true }
    }
}
