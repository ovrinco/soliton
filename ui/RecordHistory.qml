import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

SDialog {
    id: root
    property string recordId: ''
    property var versions: []
    property bool pending: false
    property string failure: ''
    property int request: 0
    readonly property bool hasDirtyDraft: (Backend.activeDraft || {}).id === recordId && !!(Backend.activeDraft || {}).dirty
    title: 'Version history'
    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(620, parent.width - 32)
    height: Math.min(620, parent.height - 32)
    modal: true; focus: true
    standardButtons: Dialog.Close
    function showFor(id) {
        recordId = id; versions = []; failure = ''; pending = true;
        var serial = ++request;
        open();
        Backend.send('history', {id:id}, (value, ok, error) => {
            if (serial !== root.request || id !== root.recordId) return;
            root.pending = false;
            if (ok) root.versions = (value || []).slice().reverse();
            else root.failure = error || 'Version history could not be read.';
        });
    }
    contentItem: ColumnLayout {
        spacing: 12
        LabelText { Layout.fillWidth: true; text: root.failure || (root.pending ? 'Reading versions…' : ''); visible: !!text; color: root.failure ? Theme.dangerText : Theme.muted; wrapMode: Text.Wrap }
        LabelText { Layout.fillWidth:true; visible:root.hasDirtyDraft; text:'Save or discard your edits before restoring a version.'; color:Theme.muted; wrapMode:Text.Wrap }
        ListView {
            id: list
            Layout.fillWidth: true; Layout.fillHeight: true
            clip: true
            model: root.versions
            delegate: Item {
                id: version
                required property var modelData
                width: list.width; height: 82
                RowLayout {
                    anchors.fill: parent; spacing: 12
                    ColumnLayout {
                        Layout.fillWidth: true; spacing: 4
                        LabelText { Layout.fillWidth: true; text: version.modelData.title; font.weight: Font.Medium }
                        LabelText { text: 'Revision ' + version.modelData.revision + '  /  ' + Qt.formatDateTime(new Date(version.modelData.updated_at*1000), 'd MMM yyyy, hh:mm'); color: Theme.muted; font.pixelSize: 12 }
                    }
                    SButton {
                        text: version.modelData.hash === (Backend.get(root.recordId) || {}).hash ? 'Current' : 'Restore'
                        enabled: !root.pending && !root.hasDirtyDraft && version.modelData.hash !== (Backend.get(root.recordId) || {}).hash
                        onClicked: {
                            var id = root.recordId; var serial = ++root.request; var draftAtRequest = JSON.stringify(Backend.activeDraft || {});
                            root.pending = true; root.failure = '';
                            Backend.send('restore', {id:id, hash:(Backend.get(id)||{}).hash, version:version.modelData.hash}, (value,ok,error) => {
                                if (serial !== root.request || id !== root.recordId) return;
                                root.pending = false;
                                if (ok) { if ((Backend.activeDraft || {}).id === id && JSON.stringify(Backend.activeDraft || {}) === draftAtRequest) Backend.reloadDraft(); root.showFor(id); }
                                else root.failure = error || 'The version could not be restored.';
                            });
                        }
                    }
                }
                Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: Theme.line }
            }
            StateView { anchors.fill: parent; visible: !root.pending && !root.failure && !root.versions.length; title: 'No saved versions' }
            ScrollBar.vertical: SScrollBar {}
        }
    }
}
