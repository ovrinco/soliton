pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

// The Modules destination: the stack and the modules it holds, side by side.
// Recovery and Settings live on the Settings page, not here.
ColumnLayout {
    id: root
    readonly property var sections: [
        {key: 'Stack', heading: 'Stack map'},
        {key: 'Modules', heading: 'Installed modules'},
        {key: 'Received files', heading: 'Received files'}
    ]
    // A section remembered from an earlier build that has left this page opens the stack map.
    property string section: root.sections.some(s => s.key === Backend.developerSection) ? Backend.developerSection : 'Stack'
    spacing: 10
    function openModule(id) { Backend.developerModule=id; Backend.developerSection='Modules'; Backend.remember(); }
    RowLayout {
        Layout.fillWidth: true
        LabelText { text: 'Modules'; font.pixelSize: 23; font.weight: Font.DemiBold }
        Rectangle { Layout.preferredWidth: 1; Layout.preferredHeight: 22; color: Theme.line }
        Flow {
            Layout.fillWidth: true; Layout.preferredHeight: implicitHeight; spacing: 4
            Repeater {
                model: root.sections
                SButton { required property var modelData; objectName: 'modules-section-' + modelData.key; text: modelData.heading; primary: root.section===modelData.key; quiet: !primary; onClicked: { Backend.developerSection=modelData.key; Backend.remember(); } }
            }
        }
    }
    Loader {
        Layout.fillWidth: true; Layout.fillHeight: true
        sourceComponent: root.section==='Modules' ? modulesComponent : root.section==='Received files' ? inboxComponent : stackComponent
    }
    Component { id: stackComponent; StackMap { onOpenModule: id=>root.openModule(id) } }
    Component { id: modulesComponent; ZetlWorkspace {} }
    Component { id: inboxComponent; InboxView {
        onArtifactRequested: receipt=>Qt.openUrlExternally('file://'+receipt.path)
        onVoiceNoteRequested: receipt=>Backend.send('history-from-inbox',{id:receipt.id},(r,ok,e)=>{if(ok)Backend.showHistory(r.record.id);else Backend.message(e,true);})
        onTranscriptRequested: receipt=>Backend.send('history-import',{path:receipt.transcript_path,provider:'manual'},(r,ok,e)=>{if(ok)Backend.showHistory(r.record.id);else Backend.message(e,true);})
    } }
}
