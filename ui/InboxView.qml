import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

Item {
    id: root

    signal voiceNoteRequested(var receipt)
    signal transcriptRequested(var receipt)
    signal artifactRequested(var receipt)

    property var items: []
    property var selectedItem: null
    property bool busy: false
    property string errorText: ''
    property string noticeText: ''
    property int requestSerial: 0
    property int listRequest: 0
    property int actionRequest: 0
    property bool splitWide: width >= 900

    readonly property var selectedReceipt: selectedItem && selectedItem.receipt ? selectedItem.receipt : null
    readonly property bool selectedCommitted: !!selectedItem && (selectedItem.state === 'committed' || !!selectedReceipt)
    readonly property bool canImportVoiceNote: selectedCommitted && !!selectedReceipt && selectedItem.source_intent === 'voice_note'
    readonly property bool canImportTranscript: selectedCommitted && !!selectedReceipt && !!selectedReceipt.transcript_path

    function nextRequest() {
        return ++root.requestSerial;
    }

    function loadList() {
        var token = root.nextRequest();
        root.listRequest = token;
        root.actionRequest = token;
        root.busy = true;
        root.errorText = '';
        Backend.send('inbox-list', { limit: 100 }, (result, ok, error) => {
            if (!root || token !== root.listRequest)
                return;
            root.busy = false;
            if (!ok) {
                root.errorText = error || 'Received files could not be loaded.';
                return;
            }
            var next = (result || {}).items || [];
            var selectedId = root.selectedItem ? String(root.selectedItem.id || '') : '';
            root.items = next;
            root.selectedItem = null;
            if (selectedId) {
                for (var i = 0; i < next.length; i++) {
                    if (String(next[i].id || '') === selectedId) {
                        root.selectedItem = next[i];
                        break;
                    }
                }
            }
        });
    }

    function selectItem(item) {
        if (!item || !item.id)
            return;
        root.selectedItem = item;
        root.noticeText = '';
        var token = root.nextRequest();
        root.actionRequest = token;
        Backend.send('inbox-status', { id: item.id }, (result, ok, error) => {
            if (!root || token !== root.actionRequest || !root.selectedItem || root.selectedItem.id !== item.id)
                return;
            if (ok)
                root.selectedItem = result;
            else
                root.errorText = error || 'The transfer status could not be refreshed.';
        });
    }

    function cancelSelected() {
        if (!root.selectedItem || root.selectedCommitted || root.busy)
            return;
        var id = root.selectedItem.id;
        var token = root.nextRequest();
        root.actionRequest = token;
        root.busy = true;
        root.errorText = '';
        root.noticeText = '';
        Backend.send('inbox-cancel', { id: id }, (result, ok, error) => {
            if (!root || token !== root.actionRequest || !root.selectedItem || root.selectedItem.id !== id)
                return;
            root.busy = false;
            if (!ok) {
                root.errorText = error || 'This transfer could not be cancelled.';
                return;
            }
            root.selectedItem = result;
            root.noticeText = 'Transfer cancelled. Any committed artifact remains preserved.';
            root.loadList();
        });
    }

    function formatBytes(value) {
        var bytes = Number(value || 0);
        if (bytes < 1024)
            return bytes + ' B';
        if (bytes < 1024 * 1024)
            return (bytes / 1024).toFixed(1) + ' KiB';
        if (bytes < 1024 * 1024 * 1024)
            return (bytes / (1024 * 1024)).toFixed(1) + ' MiB';
        return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GiB';
    }

    function progress(item) {
        if (!item || !item.size)
            return 0;
        return Math.max(0, Math.min(1, Number(item.offset || 0) / Number(item.size)));
    }

    function statusLabel(item) {
        if (!item)
            return 'Unknown';
        if (item.state === 'committed')
            return 'Saved on desktop';
        if (item.state === 'cancelled')
            return 'Cancelled';
        if (item.state === 'committing')
            return 'Verifying and saving';
        return 'Receiving · ' + Math.round(root.progress(item) * 100) + '%';
    }

    function statusColor(item) {
        if (item && item.state === 'committed')
            return Theme.green;
        if (item && item.state === 'cancelled')
            return Theme.muted;
        if (item && item.state === 'committing')
            return Theme.accent;
        return Theme.text;
    }

    function receiptValue(item, key) {
        return item && item.receipt ? item.receipt[key] : '';
    }

    Connections {
        target: Backend
        function onPrivacyApplied() { root.loadList(); }
    }
    Component.onCompleted: root.loadList()

    ColumnLayout {
        anchors.fill: parent
        spacing: 12

        RowLayout {
            Layout.fillWidth: true
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 3
                LabelText {
                    text: 'Received files'
                    font.pixelSize: Theme.titleSize
                    font.weight: Font.DemiBold
                }
                LabelText {
                    text: 'Files sent to this computer, kept exactly as they arrived.'
                    color: Theme.muted
                    font.pixelSize: 12
                    Layout.fillWidth: true
                    wrapMode: Text.Wrap
                }
            }
            LabelText {
                visible: root.busy
                text: 'Reading…'
                color: Theme.muted
            }
        }

        LabelText {
            Layout.fillWidth: true
            visible: root.errorText.length > 0
            text: root.errorText
            color: Theme.dangerText
            wrapMode: Text.Wrap
            Accessible.role: Accessible.AlertMessage
        }
        LabelText {
            Layout.fillWidth: true
            visible: root.noticeText.length > 0
            text: root.noticeText
            color: Theme.green
            wrapMode: Text.Wrap
        }

        SplitView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            orientation: root.splitWide ? Qt.Horizontal : Qt.Vertical
            handle: SSplitHandle { horizontal: root.splitWide }

            Item {
                SplitView.minimumWidth: root.splitWide ? 330 : -1
                SplitView.preferredWidth: root.splitWide ? 420 : -1
                SplitView.minimumHeight: root.splitWide ? -1 : 210
                SplitView.preferredHeight: root.splitWide ? -1 : 280

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 8
                    LabelText {
                        text: root.busy ? 'INBOX · UPDATING' : 'INBOX'
                        color: Theme.muted
                        font.pixelSize: 12
                        font.letterSpacing: 1.2
                    }
                    ListView {
                        id: inboxList
                        Layout.fillWidth: true
                        Layout.fillHeight: true
                        clip: true
                        spacing: 7
                        model: root.items
                        keyNavigationEnabled: true
                        activeFocusOnTab: true
                        Accessible.name: 'Received files'
                        delegate: Rectangle {
                            id: itemCard
                            required property var modelData
                            width: inboxList.width
                            height: itemColumn.implicitHeight + 22
                            radius: Theme.cornerRadius
                            color: root.selectedItem && root.selectedItem.id === modelData.id ? Theme.selection : (cardMouse.containsMouse ? Theme.hover : Theme.panel)
                            border.color: root.selectedItem && root.selectedItem.id === modelData.id ? Theme.accent : Theme.line
                            MouseArea {
                                id: cardMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                onClicked: root.selectItem(itemCard.modelData)
                            }
                            Keys.onEnterPressed: root.selectItem(itemCard.modelData)
                            Keys.onReturnPressed: root.selectItem(itemCard.modelData)
                            activeFocusOnTab: true
                            focus: root.selectedItem && root.selectedItem.id === modelData.id
                            Accessible.role: Accessible.ListItem
                            Accessible.name: (modelData.original_filename || modelData.filename || 'Received file') + ', ' + root.statusLabel(modelData)

                            ColumnLayout {
                                id: itemColumn
                                anchors.fill: parent
                                anchors.margins: 11
                                spacing: 5
                                RowLayout {
                                    Layout.fillWidth: true
                                    LabelText {
                                        text: modelData.original_filename || modelData.filename || 'Received file'
                                        font.pixelSize: 14
                                        font.weight: Font.Medium
                                        Layout.fillWidth: true
                                        elide: Text.ElideMiddle
                                    }
                                    LabelText {
                                        text: modelData.source_intent === 'voice_note' ? 'Voice note' : 'File'
                                        color: Theme.accent
                                        font.pixelSize: 12
                                    }
                                }
                                LabelText {
                                    text: root.statusLabel(modelData)
                                    color: root.statusColor(modelData)
                                    font.pixelSize: 12
                                    Layout.fillWidth: true
                                }
                                Rectangle {
                                    Layout.fillWidth: true
                                    height: 4
                                    radius: 2
                                    color: Theme.base
                                    visible: modelData.state !== 'committed' && modelData.state !== 'cancelled'
                                    Rectangle {
                                        width: parent.width * root.progress(itemCard.modelData)
                                        height: parent.height
                                        radius: 2
                                        color: Theme.accent
                                    }
                                }
                                LabelText {
                                    text: root.formatBytes(modelData.offset) + ' of ' + root.formatBytes(modelData.size)
                                    color: Theme.muted
                                    font.pixelSize: 12
                                }
                            }
                        }
                        ScrollBar.vertical: SScrollBar {}
                    }
                    LabelText {
                        Layout.fillWidth: true
                        visible: root.items.length === 0 && !root.busy
                        text: 'No files have reached the inbox yet.'
                        color: Theme.muted
                        wrapMode: Text.Wrap
                    }
                }
            }

            Item {
                SplitView.minimumWidth: root.splitWide ? 420 : -1
                SplitView.minimumHeight: root.splitWide ? -1 : 330
                SplitView.fillWidth: true
                SplitView.fillHeight: true

                ColumnLayout {
                    anchors.fill: parent
                    spacing: 10
                    visible: !!root.selectedItem

                    RowLayout {
                        Layout.fillWidth: true
                        LabelText {
                            text: root.selectedItem ? (root.selectedItem.original_filename || root.selectedItem.filename || 'Received file') : ''
                            font.pixelSize: 20
                            font.weight: Font.DemiBold
                            Layout.fillWidth: true
                            elide: Text.ElideMiddle
                        }
                        LabelText {
                            visible: root.busy
                            text: 'Working…'
                            color: Theme.muted
                            font.pixelSize: 12
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        color: Theme.panel
                        radius: Theme.cornerRadius
                        border.color: Theme.line
                        implicitHeight: detailColumn.implicitHeight + 24
                        ColumnLayout {
                            id: detailColumn
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 7
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Status'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                LabelText { text: root.statusLabel(root.selectedItem); color: root.statusColor(root.selectedItem); font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.Wrap }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Bytes'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                LabelText { text: root.formatBytes(root.selectedItem ? root.selectedItem.offset : 0) + ' of ' + root.formatBytes(root.selectedItem ? root.selectedItem.size : 0); font.pixelSize: 12; Layout.fillWidth: true }
                            }
                            Rectangle {
                                Layout.fillWidth: true
                                height: 5
                                radius: 2
                                color: Theme.base
                                visible: !!root.selectedItem && !root.selectedCommitted
                                Rectangle { width: parent.width * root.progress(root.selectedItem); height: parent.height; radius: 2; color: Theme.accent }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Checksum'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                Text {
                                    text: root.selectedItem ? (root.selectedItem.sha256 || root.receiptValue(root.selectedItem, 'sha256') || 'Unavailable') : ''
                                    color: Theme.text
                                    font.family: Theme.mono
                                    font.pixelSize: 12
                                    wrapMode: Text.WrapAnywhere
                                    Layout.fillWidth: true
                                    Accessible.name: 'SHA-256 checksum'
                                }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Intent'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                LabelText { text: root.selectedItem ? (root.selectedItem.source_intent || root.selectedItem.intent || 'store') : ''; font.pixelSize: 12; Layout.fillWidth: true }
                            }
                        }
                    }

                    Rectangle {
                        Layout.fillWidth: true
                        color: Theme.panel
                        radius: Theme.cornerRadius
                        border.color: Theme.line
                        implicitHeight: receiptColumn.implicitHeight + 24
                        visible: root.selectedCommitted
                        ColumnLayout {
                            id: receiptColumn
                            anchors.fill: parent
                            anchors.margins: 12
                            spacing: 7
                            LabelText { text: 'DURABLE RECEIPT'; color: Theme.muted; font.pixelSize: 12; font.letterSpacing: 1.2 }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Receipt'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                Text { text: root.receiptValue(root.selectedItem, 'receipt_id') || 'Unavailable'; color: Theme.text; font.family: Theme.mono; font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Artifact'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                Text { text: root.receiptValue(root.selectedItem, 'artifact_id') || 'Unavailable'; color: Theme.text; font.family: Theme.mono; font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere }
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                LabelText { text: 'Saved path'; color: Theme.muted; font.pixelSize: 12; Layout.preferredWidth: 105 }
                                Text { text: root.receiptValue(root.selectedItem, 'path') || 'Unavailable'; color: Theme.text; font.family: Theme.mono; font.pixelSize: 12; Layout.fillWidth: true; wrapMode: Text.WrapAnywhere; Accessible.name: 'Saved artifact path' }
                            }
                            LabelText {
                                visible: !root.canImportVoiceNote && !root.canImportTranscript
                                text: 'The original bytes are saved. Further indexing is available only when an owning source adapter is present.'
                                color: Theme.muted
                                font.pixelSize: 12
                                wrapMode: Text.Wrap
                                Layout.fillWidth: true
                            }
                            RowLayout {
                                Layout.fillWidth: true
                                visible: root.canImportVoiceNote || root.canImportTranscript
                                SButton {
                                    visible: root.canImportVoiceNote
                                    text: 'Open in Voice notes'
                                    onClicked: root.voiceNoteRequested(root.selectedReceipt)
                                    Accessible.name: 'Open received voice note in Voice notes'
                                }
                                SButton {
                                    visible: root.canImportTranscript
                                    text: 'Import transcript'
                                    onClicked: root.transcriptRequested(root.selectedReceipt)
                                    Accessible.name: 'Import received transcript'
                                }
                            }
                        }
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        SButton {
                            text: 'Open saved file'
                            enabled: root.selectedCommitted && !!root.receiptValue(root.selectedItem, 'path')
                            onClicked: root.artifactRequested(root.selectedReceipt)
                            Accessible.name: 'Open saved received file'
                        }
                        Item { Layout.fillWidth: true }
                        SButton {
                            text: 'Cancel transfer'
                            visible: !!root.selectedItem && !root.selectedCommitted && root.selectedItem.state !== 'cancelled'
                            enabled: !root.busy
                            quiet: true
                            onClicked: root.cancelSelected()
                        }
                    }
                }

                ColumnLayout {
                    anchors.centerIn: parent
                    visible: !root.selectedItem
                    spacing: 6
                    LabelText {
                        text: 'Select a received file'
                        font.pixelSize: 16
                        Layout.alignment: Qt.AlignHCenter
                    }
                    LabelText {
                        text: 'A receipt appears only after bytes are verified and saved.'
                        color: Theme.muted
                        font.pixelSize: 12
                        Layout.alignment: Qt.AlignHCenter
                    }
                }
            }
        }
    }
}
