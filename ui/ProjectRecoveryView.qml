import QtQuick
import QtQuick.Controls
import QtQuick.Dialogs
import QtQuick.Layouts

ColumnLayout {
    id:root
    property var groups:[]
    property var preview: null
    property var receipt: null
    property bool busy:false
    property string error:''
    spacing:10
    function send(c,a,done) {
        busy=true;error='';
        Backend.send(c,a,(r,ok,e)=>{if(!root)return;root.busy=false;if(!ok){root.error=e || 'Recovery operation failed';return;}done(r);});
    }
    function refresh() {send('recovery-group-list',{},r=>{if(root)root.groups=r.items || [];});}
    Component.onCompleted:refresh()
    LabelText{text:'Project checkpoints';font.pixelSize: Theme.titleSize;Layout.fillWidth:true}
    LabelText{text:'Preserve an explicit set of files together. Each file is limited to 16 MiB, with up to 256 files and 256 MiB per group.';color:Theme.muted;wrapMode:Text.Wrap;Layout.fillWidth:true}
    RowLayout {
        Layout.fillWidth:true
        Field{id:title;Layout.fillWidth:true;placeholderText:'Checkpoint label';Accessible.name:'Project checkpoint label'}
        SButton{text:'Choose files…';enabled:!root.busy;onClicked:filePicker.open()}
        LabelText{visible:root.busy;text:'Reading…';color:Theme.muted}
    }
    ScrollView {
        Layout.fillWidth:true;Layout.preferredHeight:100
        STextArea {id:paths;placeholderText:'Absolute file paths, one per line';wrapMode:TextEdit.NoWrap;Accessible.name:'Files to checkpoint';color:Theme.text;selectByMouse:true}
    }
    RowLayout {
        Layout.fillWidth:true
        SButton{text:'Checkpoint selected files';primary:true;enabled:!root.busy && !!paths.text.trim();onClicked:root.send('recovery-group-checkpoint',{title:title.text || 'Selected project files',paths:paths.text.split('\n').map(p=>p.trim()).filter(p=>p.length)},r=>{root.refresh();root.send('recovery-group-preview',{id:r.id},p=>{root.preview=p;root.receipt=null;});})}
        Choice{id:group;Layout.fillWidth:true;model:root.groups;textRole:'title';Accessible.name:'Saved project checkpoints'}
        SButton{text:'Review checkpoint';enabled:!root.busy && group.currentIndex>=0;onClicked:root.send('recovery-group-preview',{id:root.groups[group.currentIndex].id},p=>{root.preview=p;root.receipt=null;})}
    }
    LabelText{text:root.error;color:Theme.dangerText;visible:!!root.error;Layout.fillWidth:true;wrapMode:Text.Wrap}
    LabelText{text:root.preview ? root.preview.notice : 'Choose a checkpoint to compare each original with the current file.';color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap}
    ListView {
        id:files;Layout.fillWidth:true;Layout.fillHeight:true;clip:true;spacing:6;model:root.preview ? root.preview.entries : []
        ScrollBar.vertical:SScrollBar {}
        delegate:Rectangle {
            required property var modelData
            width:files.width;height:62;color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius
            ColumnLayout{anchors.fill:parent;anchors.margins:8
                LabelText{text:modelData.path;Layout.fillWidth:true;elide:Text.ElideMiddle}
                LabelText{text:modelData.missing?'Missing · recreate from checkpoint':modelData.conflict?'Changed · preserve current bytes, then restore checkpoint':'Unchanged';color:modelData.conflict?Theme.accent:Theme.muted;font.pixelSize:12}
            }
        }
    }
    LabelText {
        visible:!!root.receipt;Layout.fillWidth:true;wrapMode:Text.Wrap
        color:root.receipt && root.receipt.status==='applied'?Theme.green:Theme.danger
        text:!root.receipt?'':root.receipt.status==='applied'?'Restore complete. Every file has a durable receipt and any replaced content has its own checkpoint.':'Restore stopped after '+(root.receipt.outcomes || []).length+' files. '+(root.receipt.error || '')+' Review current files before continuing.'
    }
    SButton{text:'Restore reviewed files';enabled:!!root.preview && !root.busy;primary:true;onClicked:confirmation.open()}
    FileDialog {
        id:filePicker;title:'Choose files to checkpoint';fileMode:FileDialog.OpenFiles
        onAccepted:paths.text=selectedFiles.map(url=>decodeURIComponent(String(url).replace('file://',''))).join('\n')
    }
    SDialog {
        background:Rectangle{color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius}
        id:confirmation;title:'Restore this file group?';anchors.centerIn:parent;modal:true;width:Math.min(root.width-24,560);height:240;standardButtons:Dialog.Ok|Dialog.Cancel
        onAccepted:root.send('recovery-group-restore',{id:root.preview.id,expected_revision:root.preview.revision,op_id:root.preview.op_id},r=>{root.receipt=r;})
        LabelText{width:parent.width;wrapMode:Text.Wrap;text:'Only the listed files are restored. Current bytes are checkpointed before replacement. A later edit stops that file from being overwritten. An interruption may leave a partial restore; retrying this reviewed operation resumes using its receipts.'}
    }
}
