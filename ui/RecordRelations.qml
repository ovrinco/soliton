import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id: root
    property var record: ({})
    readonly property bool locked: !!(record.occurrence && record.occurrenceOnly)
    spacing: 10
    LabelText { text: 'Files'; font.weight: Font.Medium }
    STextArea {
        Layout.fillWidth: true
        Layout.minimumHeight: 72
        readOnly: root.locked
        text: (root.record.files || []).join('\n')
        placeholderText: 'One file path per line'
        font.family: Theme.mono
        font.pixelSize: 13
        wrapMode: TextEdit.Wrap
        onTextChanged: if (activeFocus && text !== (root.record.files || []).join('\n')) Backend.edit('files', text.split('\n').map(s => s.trim()).filter(s => s))
        Accessible.name: 'Linked file paths'
    }
    LabelText { text: 'Connections'; font.weight: Font.Medium; Layout.topMargin: 8 }
    Choice {
        Layout.fillWidth: true
        enabled: !root.locked
        model: [{ id:'', title:'Link an object…' }].concat(Backend.records.filter(r => !r.hidden && r.id !== root.record.id && (root.record.links || []).indexOf(r.id) < 0))
        textRole: 'title'
        onActivated: {
            if (index > 0) Backend.edit('links', (root.record.links || []).concat([model[index].id]));
            currentIndex = 0;
        }
        Accessible.name: 'Link an existing object'
    }
    Repeater {
        model: root.record.links || []
        RowLayout {
            id: linked
            required property string modelData
            Layout.fillWidth: true
            MenuAction {
                text: Backend.label(linked.modelData)
                onClicked: {
                    var record = Backend.get(linked.modelData);
                    if (record && record.kind === 'proposal') Backend.reviewProposalRequested(linked.modelData);
                    else Backend.select(linked.modelData);
                }
            }
            SButton {
                iconName: 'close'; accessibleLabel: 'Unlink ' + Backend.label(linked.modelData); quiet: true
                enabled: !root.locked
                onClicked: Backend.edit('links', (root.record.links || []).filter(id => id !== linked.modelData))
            }
        }
    }
    LabelText { text: 'Tags'; font.weight: Font.Medium; Layout.topMargin: 8 }
    Field {
        Layout.fillWidth: true
        enabled: !root.locked
        text: (root.record.tags || []).join(', ')
        placeholderText: 'Comma-separated tags'
        onTextEdited: Backend.edit('tags', text.split(',').map(s => s.trim()).filter(s => s))
    }
}
