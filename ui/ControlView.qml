import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs
import Quickshell
ColumnLayout {
    id: root
    signal recoveryRequested()
    property var importPreview: ({})
    property alias exportPicker: exportFile
    property alias importPicker: importFile
    property alias importReviewPanel: importReview
    spacing: 12
    SectionLabel { text: 'Workspace preferences' }
    GridLayout {
        columns: 2
        columnSpacing: 16
        rowSpacing: 12
        Layout.fillWidth: true
        LabelText { text: 'Timezone' }
        Field {
            Layout.fillWidth: true
            text: Backend.prefs.timezone
            onEditingFinished: Backend.setting('timezone', text)
        }
        LabelText { text: 'Calendar density' }
        RowLayout {
            Layout.fillWidth: true
            SSlider {
                Layout.fillWidth: true
                from: 32; to: 100; stepSize: 2
                value: Backend.prefs.hourHeight
                onMoved: Backend.setting('hourHeight', Math.round(value))
            }
            LabelText { text: String(Backend.prefs.hourHeight) + ' px/hour'; color: Theme.muted }
        }
        LabelText { text: 'Theme' }
        Choice {
            id: themeChoice
            objectName: 'settings-theme'
            Layout.fillWidth: true
            // OVRIN first and by default; every other installed theme stays selectable.
            readonly property var options: [{ id: '', title: 'OVRIN (default)' }]
                .concat(Theme.themes.filter(t => t.id !== 'ovrin').map(t => ({ id: t.id, title: t.id.split('-').map(w => w.charAt(0).toUpperCase() + w.slice(1)).join(' ') })))
                .concat([{ id: 'desktop', title: 'Follow desktop theme' }])
            model: options
            textRole: 'title'
            currentIndex: Math.max(0, options.findIndex(o => o.id === (Backend.prefs.theme || '')))
            onActivated: index => Backend.setting('theme', options[index].id)
        }
        LabelText { text: 'Motion' }
        SCheckBox {
            text: 'Reduce motion'
            checked: !!Backend.prefs.reducedMotion
            onToggled: Backend.setting('reducedMotion', checked)
        }
        LabelText { text: 'Automatic capture' }
        SettingsSwitch {
            objectName: 'settings-automatic-capture'
            Accessible.name: 'Automatic capture'
            checked: Backend.prefs.automaticCapture === true
            onClicked: Backend.setting('automaticCapture', !checked)
        }
        LabelText { text: 'Local model' }
        SettingsSwitch {
            objectName: 'settings-local-model'
            Accessible.name: 'Local model'
            checked: Backend.prefs.localModel === true
            onClicked: Backend.setting('localModel', !checked)
        }
        LabelText { text: 'Version' }
        LabelText {
            objectName: 'settings-version'
            text: (Backend.release.version || '') + (Backend.release.built_on ? ' · ' + Backend.release.built_on : '')
            color: Backend.release.matches_disk === true ? Theme.text : Theme.dangerText
        }
    }
    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; Layout.topMargin: 12; color: Theme.line }
    SectionLabel { text: 'Private' }
    LabelText { text: 'Hidden entirely while private mode is on'; color: Theme.muted }
    ColumnLayout {
        objectName: 'private-modules'
        Layout.fillWidth: true
        spacing: 4
        Repeater {
            model: [
                {key: 'planner', name: 'Goals, tasks, notes and calendar'},
                {key: 'habits', name: 'Habits'},
                {key: 'kitchen', name: 'Kitchen'},
                {key: 'finance', name: 'Finance', ledger: true},
                {key: 'history', name: 'History and voice notes'},
                {key: 'briefing', name: 'Briefings'},
                {key: 'proposals', name: 'Proposals'},
                {key: 'inbox', name: 'Inbox'},
                {key: 'collection', name: 'Collection'},
                {key: 'sheets', name: 'Sheets'},
                {key: 'qroc', name: 'Q-ROC'},
                {key: 'modules', name: 'Modules'},
                {key: 'recovery', name: 'Recovery'}
            ]
            delegate: SCheckBox {
                id: moduleMark
                required property var modelData
                objectName: 'private-module-' + modelData.key
                text: modelData.name
                // Finance is marked in the ledger itself; its row shows and
                // writes that mark, and is absent until the engine reports it.
                visible: !modelData.ledger || Backend.ledgerMarksReported
                checked: modelData.ledger ? Backend.isLedgerModulePrivate(modelData.key) : Backend.isModulePrivate(modelData.key)
                onToggled: {
                    var on = checked;
                    if (modelData.ledger) {
                        checked = Qt.binding(() => Backend.isLedgerModulePrivate(moduleMark.modelData.key));
                        Backend.setLedgerModulePrivate(modelData.key, on);
                        return;
                    }
                    checked = Qt.binding(() => Backend.isModulePrivate(moduleMark.modelData.key));
                    Backend.setModulePrivate(modelData.key, on);
                }
            }
        }
    }
    Rectangle { Layout.fillWidth: true; Layout.preferredHeight: 1; Layout.topMargin: 12; color: Theme.line }
    SectionLabel { text: 'Workspace files' }
    Flow {
        Layout.fillWidth: true
        Layout.preferredHeight: implicitHeight
        spacing: 8
        SButton { text: 'Export objects'; onClicked: exportFile.open() }
        SButton { text: 'Import…'; onClicked: importFile.open() }
        SButton { text: 'Open data folder'; onClicked: Qt.openUrlExternally('file://' + Backend.state.root) }
        SButton { objectName: 'settings-open-backups'; text: 'Open backups'; onClicked: Qt.openUrlExternally('file://' + Quickshell.env('HOME') + '/.local/lib/soliton/backups') }
        SButton { text: 'Recovery'; onClicked: root.recoveryRequested() }
    }
    Item { Layout.fillHeight: true }
    FileDialog {
        id: exportFile
        title: 'Export Soliton objects'
        fileMode: FileDialog.SaveFile
        nameFilters: ['Soliton export (*.json)']
        defaultSuffix: 'json'
        onAccepted: Backend.send('export', {
            path: decodeURIComponent(selectedFile.toString().replace('file://', ''))
        }, (r, ok) => {
            if (ok)
                Backend.message('Exported to ' + r.path);
        })
    }
    FileDialog {
        id: importFile
        title: 'Review a Soliton export'
        nameFilters: ['Soliton export (*.json)']
        onAccepted: Backend.send('import_preview', {
            path: decodeURIComponent(selectedFile.toString().replace('file://', ''))
        }, (r, ok) => {
            if (ok) {
                root.importPreview = r;
                importReview.open();
            }
        })
    }
    SPopup {
        id: importReview
        anchors.centerIn: parent
        width: Math.min(520, root.width - 20)
        height: 440
        modal: true
        focus: true
        padding: 20
        background: Rectangle {
            color: Theme.panel
            border.color: Theme.line
            radius: Theme.cornerRadius
        }
        ColumnLayout {
            anchors.fill: parent
            spacing: 12
            LabelText {
                text: 'Review workspace import'
                font.pixelSize: 21
            }
            LabelText {
                text: (root.importPreview.count || 0) + ' objects · stable identities preserved'
                color: Theme.accent
            }
            ListView {
                id: importList
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                model: root.importPreview.titles || []
                delegate: LabelText {
                    required property var modelData
                    width: importList.width
                    height: 26
                    text: modelData.kind + ' · ' + modelData.title
                }
                ScrollBar.vertical: SScrollBar {}
            }
            LabelText {
                text: 'Creates a separate workspace. Current work stays open. Exports contain saved objects; keep the data directory for full history and drafts.'
                Layout.fillWidth: true
                wrapMode: Text.Wrap
                font.pixelSize: 12
                color: Theme.muted
            }
            Field {
                id: importTarget
                Layout.fillWidth: true
                placeholderText: 'New absolute workspace directory'
            }
            RowLayout {
                Layout.fillWidth: true
                Item {
                    Layout.fillWidth: true
                }
                SButton {
                    text: 'Cancel'
                    onClicked: importReview.close()
                }
                SButton {
                    text: 'Create workspace'
                    primary: true
                    enabled: importTarget.text.startsWith('/')
                    onClicked: Backend.send('import_workspace', {
                        path: root.importPreview.source,
                        hash: root.importPreview.hash,
                        target: importTarget.text
                    }, (r, ok) => {
                        if (ok) {
                            importReview.close();
                            Backend.message('Imported ' + r.count + ' objects to ' + r.path);
                        }
                    })
                }
            }
        }
    }
}
