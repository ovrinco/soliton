import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    property var items: []
    property var selected: null
    property var preview: null
    property string error: ''
    property bool busy: false
    property string pathFilter: ''
    property string checkpointPath: ''

    function load() {
        if (!Backend.available || !Backend.restored)
            return ;

        busy = true;
        Backend.send('recovery-list', pathFilter ? {
            "path": pathFilter
        } : {
        }, (result, ok, message) => {
            if (!root) return;
            busy = false;
            if (!ok) {
                error = message || 'Recovery archive unavailable';
                return ;
            }
            error = '';
            items = result.items || [];
        }, true);
    }

    function checkpoint() {
        if (!checkpointPath)
            return ;

        busy = true;
        Backend.send('recovery-checkpoint', {
            "path": checkpointPath
        }, (result, ok, message) => {
            if (!root) return;
            busy = false;
            if (!ok) {
                error = message || 'Could not create checkpoint';
                return ;
            }
            error = '';
            selected = result;
            root.preview = null;
            load();
        }, true);
    }

    function inspect(item) {
        selected = item;
        busy = true;
        Backend.send('recovery-preview', {
            "id": item.id
        }, (result, ok, message) => {
            if (!root) return;
            busy = false;
            if (!ok) {
                error = message || 'Could not preview checkpoint';
                preview = null;
                return ;
            }
            error = '';
            preview = result;
        }, true);
    }

    function restore() {
        if (!preview)
            return ;

        busy = true;
        Backend.send('recovery-restore', {
            "id": preview.id,
            "op_id": preview.op_id,
            "expected_current_hash": preview.current_hash || 'missing'
        }, (result, ok, message) => {
            if (!root) return;
            busy = false;
            if (!ok) {
                error = message || 'Restore was not applied';
                load();
                return ;
            }
            error = '';
            preview = null;
            load();
            selected = result;
        }, true);
    }

    Component.onCompleted: root.load()
    onVisibleChanged: {
        if (visible)
            root.load();

    }

    Connections {
        function onRestoredChanged() {
            if (Backend.restored && root.visible)
                root.load();

        }

        target: Backend
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: 16
        spacing: 10

        RowLayout {
            Layout.fillWidth: true

            LabelText {
                Layout.fillWidth: true
                color: Theme.accent
                font.pixelSize: 20
                font.weight: Font.Medium
                text: 'Recovery'
            }

            LabelText {
                visible: root.busy
                text: 'Reading…'
                color: Theme.muted
            }

        }

        RowLayout {
            Layout.fillWidth: true

            Field {
                Layout.fillWidth: true
                placeholderText: 'Absolute regular file path to checkpoint'
                text: root.checkpointPath
                onTextEdited: root.checkpointPath = text
                onAccepted: root.checkpoint()
            }

            SButton {
                primary: true
                enabled: !!root.checkpointPath && !root.busy
                text: 'Checkpoint'
                onClicked: root.checkpoint()
            }

        }

        RowLayout {
            Layout.fillWidth: true

            Field {
                Layout.fillWidth: true
                placeholderText: 'Filter by exact checkpoint path'
                text: root.pathFilter
                onTextEdited: root.pathFilter = text
                onAccepted: root.load()
            }

            SButton {
                quiet: true
                text: 'Find'
                onClicked: root.load()
            }

        }

        LabelText {
            Layout.fillWidth: true
            color: Theme.dangerText
            text: root.error
            visible: !!root.error
            wrapMode: Text.Wrap
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 10

            Rectangle {
                Layout.fillWidth: true
                Layout.fillHeight: true
                color: Theme.base
                border.color: Theme.line
                radius: Theme.cornerRadius

                ListView {
                    id: checkpointList

                    anchors.fill: parent
                    anchors.margins: 6
                    clip: true
                    model: root.items
                    spacing: 3

                    ScrollBar.vertical: SScrollBar {
                    }

                    delegate: ItemDelegate {
                        required property var modelData
                        clip: true

                        width: checkpointList.width
                        height: 52
                        Accessible.name: modelData.path + ', ' + modelData.bytes + ' bytes'
                        onClicked: root.inspect(modelData)

                        background: Rectangle {
                            color: root.selected && root.selected.id === modelData.id ? Theme.selection : parent.hovered ? Theme.hover : 'transparent'
                            radius: Theme.cornerRadius
                        }

                        contentItem: Column {
                            anchors.fill: parent
                            anchors.margins: 8
                            spacing: 2

                            LabelText {
                                width: parent.width
                                color: Theme.text
                                maximumLineCount: 1; wrapMode: Text.NoWrap; elide: Text.ElideMiddle
                                text: modelData.path
                            }

                            LabelText {
                                width: parent.width
                                color: Theme.muted
                                font.pixelSize: 12
                                text: modelData.bytes + ' bytes · ' + modelData.hash
                                maximumLineCount: 1; wrapMode: Text.NoWrap; elide: Text.ElideRight
                            }

                        }

                    }

                }

                LabelText {
                    anchors.centerIn: parent
                    color: Theme.muted
                    text: root.busy ? 'Reading checkpoint…' : 'No checkpoints'
                    visible: !root.items.length
                }

            }

            Rectangle {
                Layout.preferredWidth: 360
                Layout.fillHeight: true
                color: Theme.base
                border.color: Theme.line
                radius: Theme.cornerRadius

                ColumnLayout {
                    anchors.fill: parent
                    anchors.margins: 12
                    spacing: 8

                    LabelText {
                        Layout.fillWidth: true
                        color: Theme.accent
                        font.pixelSize: 13
                        text: root.preview ? 'Review before restore' : 'Select a checkpoint'
                    }

                    LabelText {
                        Layout.fillWidth: true
                        color: Theme.text
                        text: root.preview ? root.preview.path : 'No file selected'
                        wrapMode: Text.Wrap
                    }

                    LabelText {
                        Layout.fillWidth: true
                        color: root.preview && root.preview.conflict ? Theme.danger : Theme.muted
                        text: root.preview ? (root.preview.current_state === 'missing' ? 'Current file is missing · expected hash: missing' : root.preview.conflict ? 'Current file differs from this checkpoint. Restoring preserves a checkpoint of the current bytes first.' : 'Current file matches the reviewed preview') : 'Review the current hash and diff here.'
                        wrapMode: Text.Wrap
                    }

                    LabelText {
                        Layout.fillWidth: true
                        color: Theme.muted
                        font.family: Theme.mono
                        font.pixelSize: 12
                        text: root.preview ? 'Current: ' + (root.preview.current_hash || 'missing') + '\nCheckpoint: ' + root.preview.checkpoint_hash : ''
                        wrapMode: Text.Wrap
                    }

                    STextArea {
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        color: Theme.text
                        font.family: Theme.mono
                        font.pixelSize: 12
                        readOnly: true
                        text: root.preview && root.preview.diff ? JSON.stringify(root.preview.diff, null, 2) : 'No inline diff available.'
                        wrapMode: TextEdit.Wrap

                        background: Rectangle {
                            color: Theme.panel
                            radius: Theme.cornerRadius
                        }

                    }

                    RowLayout {
                        Layout.fillWidth: true

                        SButton {
                            enabled: !!root.preview && !root.busy
                            primary: true
                            text: 'Restore reviewed file'
                            onClicked: restoreReview.open()
                        }

                        SButton {
                            quiet: true
                            text: 'Cancel'
                            onClicked: {
                                root.preview = null;
                                root.selected = null;
                            }
                        }

                    }

                }

            }

        }

    }

    SDialog {
        id: restoreReview
        title: 'Restore reviewed file?'
        anchors.centerIn: parent
        width: Math.min(root.width-32, 500)
        modal: true
        height: 230
        standardButtons: Dialog.Ok | Dialog.Cancel
        onAccepted: root.restore()
        LabelText {
            width: parent.width
            wrapMode: Text.Wrap
            text: 'The current file will be preserved as a new checkpoint before replacement. If it has changed since this preview, restoration stops. Other files and external effects are outside this restore.'
        }
    }

}
