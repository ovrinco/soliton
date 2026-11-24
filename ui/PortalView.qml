pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Layouts
import Quickshell
import Quickshell.Io

ColumnLayout {
    id: root
    spacing: 12
    // The view is a route, so the side panel and the palette reach it.
    readonly property string activeTab: Backend.page === 'Portal' && ['Devices', 'Transfers', 'Media'].indexOf(Backend.view) >= 0 ? Backend.view : 'Devices'
    property var devices: []
    property bool devicesConfigured: false
    property string deviceError: ''
    function loadDevices() { if (!deviceReader.running) deviceReader.running = true; }
    function contactTime(seconds) {
        return seconds ? Qt.formatDateTime(new Date(seconds * 1000), 'MMM d, yyyy h:mm ap') : 'No contact recorded';
    }
    function openReceipt(receipt) {
        if (receipt && receipt.path)
            Qt.openUrlExternally('file://' + receipt.path)
    }
    function openVoiceNote(receipt) {
        if (!receipt || !receipt.id)
            return
        Backend.send('history-from-inbox', { id: receipt.id }, function(result, ok, error) {
            if (ok && result && result.record && result.record.id)
                Backend.showHistory(result.record.id)
            else
                Backend.message(error || 'The received voice note could not be opened.', true)
        })
    }
    Process {
        id: deviceReader
        command: ['python3', Quickshell.env('SOLITON_RESOURCE_ROOT') + '/scripts/portal_devices.py']
        stdout: StdioCollector {
            onStreamFinished: {
                try {
                    var response = JSON.parse(text);
                    root.deviceError = response.error || '';
                    if (!response.error) {
                        root.devices = response.result.items || [];
                        root.devicesConfigured = !!response.result.configured;
                    }
                } catch (error) { root.deviceError = 'Companion device state could not be read.'; }
            }
        }
    }
    // Devices are read each time the view is shown. Last contact is refreshed
    // periodically only once a companion connection is configured; without one
    // there is nothing to refresh, so no reader is started while it stays open.
    readonly property bool devicesShown: root.visible && root.activeTab === 'Devices'
    onDevicesShownChanged: if (devicesShown) root.loadDevices()
    Timer {
        interval: 10000
        running: root.devicesShown && root.devicesConfigured
        repeat: true
        onTriggered: root.loadDevices()
    }
    property var mediaItems: []
    property bool mediaLoading: false
    property string mediaError: ''
    property int mediaRequest: 0

    function formatBytes(value) {
        var bytes = Number(value || 0)
        if (bytes < 1024) return bytes + ' B'
        if (bytes < 1024 * 1024) return (bytes / 1024).toFixed(1) + ' KiB'
        if (bytes < 1024 * 1024 * 1024) return (bytes / (1024 * 1024)).toFixed(1) + ' MiB'
        return (bytes / (1024 * 1024 * 1024)).toFixed(2) + ' GiB'
    }

    function loadMedia() {
        var request = ++root.mediaRequest
        root.mediaLoading = true
        root.mediaError = ''
        Backend.send('inbox-list', { limit: 100 }, function(result, ok, error) {
            if (!root || request !== root.mediaRequest) return
            root.mediaLoading = false
            if (!ok) {
                root.mediaError = error || 'Received files could not be read.'
                root.mediaItems = []
                return
            }
            var next = []
            var items = (result || {}).items || []
            for (var i = 0; i < items.length; ++i) {
                var item = items[i]
                var name = String(item.original_filename || item.filename || '')
                var intent = String(item.source_intent || '').toLowerCase()
                if (intent.indexOf('media') >= 0 || /\.(aac|flac|m4a|mp3|mp4|mkv|mov|ogg|opus|png|jpg|jpeg|webm|wav)$/i.test(name))
                    next.push(item)
            }
            root.mediaItems = next
        })
    }

    STabBar {
        visible: Theme.pageTabs
        Layout.fillWidth: true
        Repeater {
            model: ['Devices', 'Transfers', 'Media']
            delegate: STabButton {
                required property string modelData
                text: modelData
                active: root.activeTab === modelData
                onClicked: Backend.navigate('Portal', modelData)
            }
        }
        Item { Layout.fillWidth: true }
    }

    StackLayout {
        Layout.minimumWidth: 0; Layout.minimumHeight: 0
        Layout.fillWidth: true
        Layout.fillHeight: true
        currentIndex: Math.max(0, ['Devices', 'Transfers', 'Media'].indexOf(root.activeTab))

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 12
            // The global header names the page and the side panel the view:
            // the list starts at the top of the page area, with no title band.
            ListView {
                id: deviceList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 4
                model: root.devices
                RetainedScroll { flickable: deviceList; positionKey: 'portal-devices'; active: root.visible && root.activeTab === 'Devices' }
                StateView {
                    anchors.fill: parent
                    visible: root.devices.length === 0
                    title: root.deviceError ? 'Devices unavailable' : deviceReader.running ? 'Reading devices…' : root.devicesConfigured ? 'No enrolled devices' : 'No companion connection'
                    detail: root.deviceError || (deviceReader.running ? '' : 'Enrolled devices appear here when a companion connection is configured.')
                    error: !!root.deviceError
                }
                delegate: Rectangle {
                    id: deviceRow
                    required property var modelData
                    width: ListView.view.width
                    height: 72
                    color: Theme.panel
                    border.color: Theme.line
                    RowLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        ColumnLayout {
                            Layout.fillWidth: true
                            LabelText { text: deviceRow.modelData.label; Layout.fillWidth: true; elide: Text.ElideRight }
                            LabelText { text: 'Last contact: ' + root.contactTime(deviceRow.modelData.last_seen); color: Theme.muted; font.pixelSize: 12 }
                        }
                        LabelText { text: deviceRow.modelData.state; color: deviceRow.modelData.state === 'Enrolled' ? Theme.text : Theme.muted }
                    }
                }
            }
        }

        InboxView {
            Layout.fillWidth: true
            Layout.fillHeight: true
            onArtifactRequested: receipt => root.openReceipt(receipt)
            onVoiceNoteRequested: receipt => root.openVoiceNote(receipt)
        }

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 12
            ListView {
                id: mediaList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                spacing: 6
                model: root.mediaItems
                RetainedScroll { flickable: mediaList; positionKey: 'portal-media'; active: root.visible && root.activeTab === 'Media' }
                StateView {
                    anchors.fill: parent
                    visible: root.mediaItems.length === 0
                    title: root.mediaError ? 'Media unavailable' : root.mediaLoading ? 'Reading media…' : 'No received media'
                    detail: root.mediaError || (root.mediaLoading ? '' : 'Files appear here after they have reached the inbox.')
                    error: !!root.mediaError
                }
                delegate: Rectangle {
                    id: mediaCard
                    required property var modelData
                    width: ListView.view.width
                    height: mediaRow.implicitHeight + 20
                    color: Theme.panel
                    border.color: Theme.line
                    radius: Theme.cornerRadius
                    ColumnLayout {
                        id: mediaRow
                        anchors.fill: parent
                        anchors.margins: 10
                        spacing: 3
                        RowLayout {
                            Layout.fillWidth: true
                            LabelText {
                                Layout.fillWidth: true
                                text: mediaCard.modelData.original_filename || mediaCard.modelData.filename || 'Admitted media'
                                font.weight: Font.DemiBold
                                elide: Text.ElideMiddle
                            }
                            SButton {
                                visible: !!(mediaCard.modelData.receipt && mediaCard.modelData.receipt.path)
                                text: 'Open'
                                quiet: true
                                onClicked: root.openReceipt(mediaCard.modelData.receipt)
                                Accessible.name: 'Open saved media file'
                            }
                        }
                        LabelText {
                            text: root.formatBytes(mediaCard.modelData.size) + ' · ' + String(mediaCard.modelData.state || 'unknown')
                            color: Theme.muted
                            font.pixelSize: 12
                        }
                    }
                }
            }

        }
    }

    NoticePopup { scope: root; text: root.activeTab === 'Devices' && root.devices.length ? root.deviceError : root.activeTab === 'Media' && root.mediaItems.length ? root.mediaError : '' }
    Connections {
        target: Backend
        function onPrivacyApplied() { root.loadMedia(); }
    }
    Component.onCompleted: { root.loadMedia(); if (root.devicesShown) root.loadDevices(); }
    onActiveTabChanged: if (root.activeTab === 'Media') root.loadMedia()
}
