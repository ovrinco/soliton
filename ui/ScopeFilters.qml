import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

SPopup {
    id: root
    property string scopeName: 'this view'
    property bool taskProperties: false
    parent: Overlay.overlay
    x: Math.max(12, (parent.width-width)/2)
    y: Math.max(12, (parent.height-height)/2)
    width: Math.min(460, parent.width-24)
    height: Math.min(520, parent.height-24)
    padding: 20
    modal: true
    focus: true
    contentItem: ColumnLayout {
        spacing: 12
        RowLayout {
            Layout.fillWidth: true
            LabelText { Layout.fillWidth: true; text: 'Filter ' + root.scopeName; font.pixelSize: 20; font.weight: Font.Medium }
            SButton { iconName: 'close'; accessibleLabel: 'Close filters'; quiet: true; onClicked: root.close() }
        }
        SSearchField { Layout.fillWidth: true; text: Backend.search; scopeName: root.scopeName; onTextEdited: Backend.search = text }
        RowLayout {
            visible: root.taskProperties
            Layout.fillWidth: true
            Choice { Layout.fillWidth: true; model: ['all','high','medium','low']; displayText: Backend.priority === 'all' ? 'Any priority' : Backend.priority; currentIndex: model.indexOf(Backend.priority); onActivated: Backend.priority = model[index] }
            Choice { Layout.fillWidth: true; model: ['all','todo','in_progress','completed']; displayText: ['Any status','To do','In progress','Completed'][Math.max(0, currentIndex)]; currentIndex: model.indexOf(Backend.status); onActivated: Backend.status = model[index] }
        }
        ListView {
            id: choices
            Layout.fillWidth: true; Layout.fillHeight: true
            clip: true
            model: Backend.categories.reduce((a,c) => a.concat([c]).concat(Backend.goals.filter(g => g.category_id === c.id)), [])
            delegate: SCheckBox {
                required property var modelData
                width: choices.width
                text: modelData.title
                leftPadding: padding + (modelData.kind === 'goal' ? Theme.space12 : 0)
                checked: Backend.branches.indexOf(modelData.id) >= 0
                onToggled: Backend.toggleBranch(modelData.id)
            }
            ScrollBar.vertical: SScrollBar {}
        }
        SDisclosure {
            visible: root.taskProperties
            Layout.fillWidth: true; title: 'Saved views'
            Choice {
                Layout.fillWidth:true
                model:['Choose a saved view…'].concat((Backend.prefs.savedViews || []).map(v=>v.name))
                onActivated: { if(index>0) Backend.applyScope(Backend.prefs.savedViews[index-1]); }
            }
            RowLayout {
                Layout.fillWidth:true
                Field {id:scopeNameField;Layout.fillWidth:true;placeholderText:'View name';onAccepted:{if(text.trim()){Backend.saveView(text.trim());text='';}}}
                SButton {text:'Save';enabled:!!scopeNameField.text.trim();onClicked:{Backend.saveView(scopeNameField.text.trim());scopeNameField.text='';}}
            }
        }
        SCheckBox { text: 'Show archived'; checked: Backend.showArchived; onToggled: { Backend.showArchived = checked; Backend.remember(); } }
        RowLayout {
            Layout.fillWidth: true
            SButton { text: 'Clear'; quiet: true; onClicked: Backend.clearFilters() }
            Item { Layout.fillWidth: true }
            SButton { text: 'Done'; primary: true; onClicked: root.close() }
        }
    }
}
