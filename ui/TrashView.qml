pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Trash, shown inside Archive: deleted goals, tasks, categories, kitchen profile
// entries and collection sources, each with when it was deleted and a Restore action. Every row is read from the engine's
// planner-trash-list; nothing is counted or kept here.
Item {
    id: root
    objectName: 'trash-view'
    property var entries: []
    property bool loaded: false
    property string restoringId: ''
    // The kinds this page lists; empty lists every kind. Restore routing is the same for all.
    property var kinds: []
    onKindsChanged: if (loaded) reload()
    implicitHeight: content.implicitHeight

    function reload() {
        Backend.send('planner-trash-list', {}, function(value, ok) {
            if (!ok) return;
            root.entries = (value || []).filter(e => !root.kinds.length || root.kinds.indexOf(e.kind) >= 0)
                .sort((a, b) => b.deleted_at - a.deleted_at);
            root.loaded = true;
        });
    }
    // The engine names each kind it holds; the planner kinds keep their own word if it does not.
    function kindWord(entry) {
        return entry.word || (entry.kind === 'category' ? 'Category' : entry.kind === 'goal' ? 'Goal' : 'Task');
    }
    function deletedWords(seconds) {
        return 'Deleted ' + Qt.formatDateTime(new Date(seconds * 1000), 'd MMM yyyy, HH:mm');
    }
    function restore(entry) {
        if (restoringId) return;
        restoringId = entry.id;
        // The row leaving the Trash is the confirmation; a notice here would cover the next row's Restore.
        Backend.send('planner-trash-restore', {kind: entry.kind, id: entry.id}, function() {
            root.restoringId = '';
            root.reload();
        });
    }
    Component.onCompleted: reload()
    // Any change to the planner (a delete, an Undo, a restore) refreshes the list.
    Connections {
        target: Backend
        function onStateChanged() { Qt.callLater(root.reload); }
    }

    ColumnLayout {
        id: content
        width: parent.width
        height: parent.height
        spacing: 8
        Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; color: Theme.line }
        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: 4
            LabelText { text: 'Trash'; font.pixelSize: 15; font.weight: Font.DemiBold }
            LabelText { text: root.entries.length ? String(root.entries.length) : ''; color: Theme.muted; font.pixelSize: Theme.fontSizeSecondary }
            Item { Layout.fillWidth: true }
        }
        LabelText {
            Layout.fillWidth: true
            visible: root.loaded && !root.entries.length
            color: Theme.muted
            text: 'Trash is empty'
        }
        ListView {
            id: trashList
            objectName: 'trash-list'
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.preferredHeight: contentHeight
            visible: root.entries.length > 0
            clip: true
            spacing: 4
            boundsBehavior: Flickable.StopAtBounds
            model: root.entries
            ScrollBar.vertical: SScrollBar {}
            delegate: Rectangle {
                id: row
                objectName: 'trash-row'
                required property var modelData
                width: trashList.width
                height: 52
                radius: Theme.cornerRadius
                color: Theme.panel
                Accessible.role: Accessible.ListItem
                Accessible.name: root.kindWord(modelData) + ' ' + (modelData.title || '') + ', ' + root.deletedWords(modelData.deleted_at)
                RowLayout {
                    anchors.fill: parent; anchors.leftMargin: 14; anchors.rightMargin: 8; spacing: 12
                    LabelText { Layout.preferredWidth: 72; text: root.kindWord(row.modelData); color: Theme.muted; font.pixelSize: Theme.fontSizeSecondary }
                    LabelText { Layout.fillWidth: true; text: row.modelData.title || 'Untitled'; elide: Text.ElideRight; maximumLineCount: 1 }
                    LabelText { text: root.deletedWords(row.modelData.deleted_at); color: Theme.muted; font.pixelSize: Theme.fontSizeSecondary }
                    SButton {
                        objectName: 'trash-restore'
                        text: 'Restore'
                        quiet: true
                        busy: root.restoringId === row.modelData.id
                        enabled: !root.restoringId
                        accessibleLabel: 'Restore ' + (row.modelData.title || root.kindWord(row.modelData))
                        onClicked: root.restore(row.modelData)
                    }
                }
            }
        }
    }
}
