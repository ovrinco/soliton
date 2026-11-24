import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Window

Item {
    id:root
    readonly property Item dialogParent: root.Window.window ? root.Window.window.contentItem : root
    property bool saving:false
    property string error:''
    property var review:({})
    property var staleContext:null
    function update(key,value) { var d=Object.assign({},Backend.proposalDraft);d[key]=value;Backend.proposalDraft=d;Backend.remember(); }
    function showReview(id) { root.error='';Backend.send('proposal-detail',{id:id},(r,ok,e)=>{if(ok){root.review=r;overlap.checked=false;reviewer.open();}else Backend.message(e || 'Could not open proposal',true);}); }
    function decide(action) {
        if(action==='approve' && root.review.goal_id && Object.values(Backend.drafts).some(d=>d.id===root.review.goal_id && d.dirty)){root.error='The linked goal has an unsaved desktop draft. Save or discard that draft before approval.';return;}
        root.saving=true;root.error='';
        Backend.send('proposal-decide',{id:root.review.id,expected_revision:root.review.revision,op_id:root.review.op_id,decision:action,allow_overlap:overlap.checked},(r,ok,e)=>{
            root.saving=false;
            if(ok){root.review=r;Backend.refreshProposals();Backend.message(action==='approve'?'Approved work saved to the planner.':action==='defer'?'Proposal deferred.':'Proposal rejected.');}
            else root.error=e || 'Decision was not applied. Your proposal is preserved.';
        });
    }
    function openEditor(p, sourceRevision, excerpt) {
        var sc=p.schedule || {};
        Backend.proposalDraft={editing_id:p.id,expected_revision:p.revision,source_id:p.source_id,source_revision:String(sourceRevision),segment_id:p.segment_id,source_title:p.source_title,excerpt:excerpt,title:p.title,rationale:p.rationale,goal_id:p.goal_id,date:p.date,time:sc.time || '',duration_minutes:sc.duration_minutes || 30,timezone:sc.timezone || Backend.prefs.timezone,fold:sc.fold || 'earlier'};
        Backend.remember();
        reviewer.close();
        staleReviewer.close();
        composer.open();
    }
    function editProposal(p) {
        if(Backend.proposalDraft.source_id && Backend.proposalDraft.title){root.error='Save or discard your existing proposal draft before editing another.';return;}
        root.saving=true;
        Backend.send('history-detail',{id:p.source_id},(source,ok,e)=>{
            root.saving=false;
            if(!ok){root.error=e || 'The current source could not be loaded.';return;}
            var segment=(source.segments || []).find(s=>String(s.id)===String(p.segment_id));
            if(String(source.revision)!==String(p.source_revision)){
                root.staleContext={proposal:p,source:source,segment:segment || null};
                staleReviewer.open();
                return;
            }
            if(!segment){root.error='The cited passage is no longer available. Open the current source and choose a new passage before editing.';return;}
            root.openEditor(p,source.revision,segment ? segment.text : p.excerpt);
        });
    }
    function saveProposal() {
        root.saving=true;root.error='';
        var d=Backend.proposalDraft;
        if(d.goal_id && Object.values(Backend.drafts).some(g=>g.id===d.goal_id && g.dirty)){root.saving=false;root.error='The selected goal has an unsaved desktop draft. Save or discard it before proposing work.';return;}
        if(!d.editing_id && !d.request_id){
            Backend.send('proposal-draft-id',{},(r,ok,e)=>{if(!ok){root.saving=false;root.error=e;return;}root.update('request_id',r.request_id);root.saveProposal();});
            return;
        }
        Backend.flushPreferences(ok=>{
            if(!ok){root.saving=false;root.error='The draft is still saving or the service is unavailable. Retry once it finishes; your text is preserved.';return;}
            Backend.send(d.editing_id?'proposal-revise':'proposal-create',{request_id:d.request_id || '',id:d.editing_id || '',expected_revision:d.expected_revision || '',source_id:d.source_id,source_revision:d.source_revision,segment_id:d.segment_id,title:d.title,rationale:d.rationale || '',goal_id:d.goal_id || '',date:d.date || '',schedule:d.time?{time:d.time,duration_minutes:d.duration_minutes || 30,timezone:d.timezone || Backend.prefs.timezone,fold:d.fold || 'earlier'}:null},(r,ok,e)=>{
                root.saving=false;
                if(ok){Backend.proposalDraft={};Backend.remember();composer.close();Backend.refreshProposals();root.review=r;reviewer.open();}
                else root.error=e || 'Could not save proposal';
            });
        });
    }
    Connections {
        target:Backend
        function onComposeProposalRequested(){root.error='';composer.open();}
        function onReviewProposalRequested(proposalId){root.showReview(proposalId);}
    }
    SDialog {
        id:composer
        parent:root.dialogParent; anchors.centerIn:parent
        width:Math.min(740,parent.width-40);height:Math.min(760,parent.height-50)
        modal:true;title:'Propose work from this passage'
        dirty:root.saving || Object.keys(Backend.proposalDraft).length > 0
        keepsDraft:!root.saving
        onDirtyDismissalRequested: if(!root.saving)proposalCloseGuard.open()
        background:Rectangle { color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius }
        contentItem:ScrollView {
            id:composeScroll;clip:true;contentWidth:availableWidth
            ColumnLayout {
                width:composeScroll.availableWidth;spacing:12
                LabelText { text:Backend.proposalDraft.source_title || '';textFormat:Text.PlainText;color:Theme.accent;Layout.fillWidth:true;wrapMode:Text.Wrap }
                LabelText { text:Backend.proposalDraft.excerpt || '';textFormat:Text.PlainText;Layout.fillWidth:true;wrapMode:Text.Wrap;maximumLineCount:6;color:Theme.muted;font.pixelSize:13 }
                LabelText { text:'Outcome';font.pixelSize:12 }
                Field { Layout.fillWidth:true;text:Backend.proposalDraft.title || '';placeholderText:'A concrete action you want to take';Accessible.name:'Proposed task title';onTextEdited:root.update('title',text) }
                LabelText { text:'Why this matters';font.pixelSize:12 }
                STextArea { Layout.fillWidth:true;Layout.preferredHeight:90;text:Backend.proposalDraft.rationale || '';color:Theme.text;wrapMode:TextEdit.Wrap;selectByMouse:true;Accessible.name:'Proposal rationale';background:Rectangle {color:Theme.base;border.color:Theme.line;radius: Theme.cornerRadius} onTextChanged:if(activeFocus)root.update('rationale',text) }
                LabelText { text:'Goal';font.pixelSize:12 }
                Choice { Layout.fillWidth:true;model:[{id:'',title:'Unassigned'}].concat(Backend.goals);textRole:'title';currentIndex:Math.max(0,model.findIndex(g=>g.id===Backend.proposalDraft.goal_id));Accessible.name:'Proposal goal';onActivated:root.update('goal_id',model[currentIndex].id) }
                LabelText { text:'Date and optional time';font.pixelSize:12 }
                RowLayout {
                    Layout.fillWidth:true
                    Field { Layout.fillWidth:true;text:Backend.proposalDraft.date || '';placeholderText:'YYYY-MM-DD or unscheduled';Accessible.name:'Proposal date';onTextEdited:root.update('date',text) }
                    Field { Layout.preferredWidth:100;text:Backend.proposalDraft.time || '';placeholderText:'HH:mm';Accessible.name:'Proposal time';onTextEdited:root.update('time',text) }
                    SpinBox { from:1;to:10080;value:Backend.proposalDraft.duration_minutes || 30;editable:true;Accessible.name:'Duration in minutes';onValueModified:root.update('duration_minutes',value) }
                }
                LabelText { text:'Minutes · '+(Backend.proposalDraft.timezone || Backend.prefs.timezone)+' · The planner validates this local time.';color:Theme.muted;font.pixelSize: 12;wrapMode:Text.Wrap;Layout.fillWidth:true }
                Choice { model:['Earlier repeated hour','Later repeated hour'];currentIndex:Backend.proposalDraft.fold==='later'?1:0;onActivated:root.update('fold',currentIndex===1?'later':'earlier');Accessible.name:'Daylight saving repeated hour';visible:!!Backend.proposalDraft.time }
                LabelText { visible:!!root.error;text:root.error;color:Theme.dangerText;wrapMode:Text.Wrap;Layout.fillWidth:true }
                LabelText { text:'Saving a proposal does not create a task. Review and approve it separately.';color:Theme.muted;font.pixelSize:12;wrapMode:Text.Wrap;Layout.fillWidth:true }
            }
        }
        footer:RowLayout {
            spacing:8
            SButton { text:'Discard draft';enabled:!root.saving;onClicked:{Backend.proposalDraft={};Backend.remember();composer.close();} }
            SButton { text:'Keep for later';enabled:!root.saving;onClicked:composer.close() }
            Item { Layout.fillWidth:true }
            SButton {
                busy: root.saving

                text:root.saving?'Saving…':'Save proposal';primary:true;enabled:!root.saving && !!Backend.proposalDraft.title
                onClicked:root.saveProposal()
            }
        }
    }
    SDialog {
        id:proposalCloseGuard
        parent:root.dialogParent;anchors.centerIn:parent
        width:Math.min(420,parent.width-40)
        modal:true;title:'Save proposal draft?'
        contentItem:LabelText {text:'Save the proposal, discard this draft, or continue editing.';wrapMode:Text.Wrap;padding:16}
        footer:RowLayout {
            SButton {text:'Cancel';onClicked:proposalCloseGuard.close()}
            Item {Layout.fillWidth:true}
            SButton {text:'Discard';onClicked:{Backend.proposalDraft={};Backend.remember();proposalCloseGuard.close();composer.close()}}
            SButton {text:'Save';primary:true;enabled:!!Backend.proposalDraft.title;onClicked:{proposalCloseGuard.close();root.saveProposal()}}
        }
    }
    SDialog {
        id:reviewer
        parent:root.dialogParent;anchors.centerIn:parent
        width:Math.min(740,parent.width-40);height:Math.min(720,parent.height-50)
        modal:true;title:'Review proposed work';dirty:root.saving
        background:Rectangle { color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius }
        contentItem:ScrollView {
            id:reviewScroll;clip:true;contentWidth:availableWidth
            ColumnLayout {
                width:reviewScroll.availableWidth;spacing:14
                LabelText { text:root.review.title || '';textFormat:Text.PlainText;font.pixelSize:23;font.weight:Font.DemiBold;wrapMode:Text.Wrap;Layout.fillWidth:true }
                LabelText { text:(root.review.status || '')+' · '+(root.review.date || 'Unscheduled')+(root.review.schedule?' '+root.review.schedule.time+' · '+root.review.schedule.duration_minutes+' minutes':'');color:Theme.accent;wrapMode:Text.Wrap;Layout.fillWidth:true }
                LabelText { text:root.review.rationale || 'No rationale supplied.';textFormat:Text.PlainText;wrapMode:Text.Wrap;Layout.fillWidth:true }
                LabelText { text:'SOURCE · '+(root.review.source_title || '');textFormat:Text.PlainText;color:Theme.muted;font.pixelSize:12;wrapMode:Text.Wrap;Layout.fillWidth:true }
                ScrollView {
                    id:sourceExcerpt;Layout.fillWidth:true;Layout.preferredHeight:160;clip:true;contentWidth:availableWidth
                    LabelText { width:sourceExcerpt.availableWidth;text:root.review.excerpt || '';textFormat:Text.PlainText;wrapMode:Text.Wrap;font.pixelSize:14 }
                }
                SCheckBox { id:overlap;text:'Allow an intentional overlap with existing work';visible:['ready','deferred','applying'].includes(root.review.status) && !!root.review.schedule;enabled:!root.saving }
                LabelText { visible:!!root.error;text:root.error;color:Theme.dangerText;wrapMode:Text.Wrap;Layout.fillWidth:true }
                LabelText { visible:root.review.status==='applied';text:'Saved through the same journaled writer as your planner. Repeating this approval returns the existing task.';color:Theme.green;wrapMode:Text.Wrap;Layout.fillWidth:true }
                RowLayout {
                    SButton { text:'Edit proposal';visible:['ready','deferred'].includes(root.review.status);enabled:!root.saving;onClicked:root.editProposal(root.review) }
                    SButton { text:'Original source';enabled:!root.saving;onClicked:{reviewer.close();Backend.showHistory(root.review.source_id);} }
                    SButton { text:'Preview in calendar';visible:!!root.review.date;enabled:!root.saving;onClicked:{Backend.navigate('Dashboard','Day');Backend.selectedDate=root.review.date;Backend.closeDetails();reviewer.close();} }
                    SButton { text:'Open task';visible:root.review.status==='applied';enabled:!root.saving;onClicked:{Backend.navigate('Tasks','Tasks');Backend.select(root.review.task_id);reviewer.close();} }
                }
            }
        }
        footer:RowLayout {
            SButton { text:'Close';enabled:!root.saving;onClicked:reviewer.close() }
            Item { Layout.fillWidth:true }
            SButton { text:'Reject';visible:['ready','deferred','applying'].includes(root.review.status);enabled:!root.saving;onClicked:root.decide('reject') }
            SButton { text:'Defer';visible:['ready','deferred'].includes(root.review.status);enabled:!root.saving;onClicked:root.decide('defer') }
            SButton {
                busy: root.saving
 text:root.saving?'Applying…':root.review.status==='applying'?'Reconcile approval':'Approve this task';primary:true;visible:['ready','deferred','applying'].includes(root.review.status);enabled:!root.saving;onClicked:root.decide('approve') }
        }
    }
    SDialog {
        id:staleReviewer
        parent:root.dialogParent;anchors.centerIn:parent
        width:Math.min(760,parent.width-40);height:Math.min(680,parent.height-50)
        modal:true;title:'Source changed · review before editing';dirty:root.saving
        background:Rectangle { color:Theme.panel;border.color:Theme.accent;radius: Theme.cornerRadius }
        contentItem:ScrollView {
            clip:true;contentWidth:availableWidth
            ColumnLayout { width:staleReviewer.width-36;spacing:12
                LabelText { text:'This proposal cites an older source revision.';font.pixelSize:20;font.weight:Font.DemiBold;Layout.fillWidth:true;wrapMode:Text.Wrap }
                LabelText { text:'The saved proposal remains unchanged. Compare the original passage with the current source, then explicitly choose whether to edit against the current revision.';color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap }
                LabelText { text:'Original excerpt';font.weight:Font.DemiBold;Layout.fillWidth:true }
                LabelText { text:root.staleContext ? root.staleContext.proposal.excerpt : '';textFormat:Text.PlainText;color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap }
                LabelText { text:'Current excerpt';font.weight:Font.DemiBold;Layout.fillWidth:true }
                LabelText { text:root.staleContext && root.staleContext.segment ? root.staleContext.segment.text : 'The cited segment is no longer available in the current revision.';textFormat:Text.PlainText;color:root.staleContext && root.staleContext.segment ? Theme.text : Theme.danger;Layout.fillWidth:true;wrapMode:Text.Wrap }
                LabelText { text:root.staleContext ? ('Revision '+root.staleContext.proposal.source_revision+' → '+root.staleContext.source.revision) : '';color:Theme.accent;Layout.fillWidth:true }
            }
        }
        footer:RowLayout {
            SButton { text:'Open current source';quiet:true;enabled:!root.saving;onClicked:{var id=root.staleContext ? root.staleContext.source.id : '';staleReviewer.close();Backend.showHistory(id);} }
            Item { Layout.fillWidth:true }
            SButton { text:'Cancel';enabled:!root.saving;onClicked:staleReviewer.close() }
            SButton { text:'Review current revision';primary:true;enabled:!root.saving && !!(root.staleContext && root.staleContext.segment);onClicked:{var c=root.staleContext;root.openEditor(c.proposal,c.source.revision,c.segment.text);} }
        }
    }
}
