import QtQuick
import QtQuick.Controls
import QtQuick.Layouts

ColumnLayout {
    id:root
    property bool linked:false
    property string status:'all'
    spacing:12
    Component.onCompleted: Backend.refreshProposals(0,root.status)
    onVisibleChanged: if (visible) Backend.refreshProposals(Backend.proposalOffset, root.status)
    Connections { target:Backend; function onRecordsChanged(){if(root.visible)Backend.refreshProposals(Backend.proposalOffset,root.status);} }
    RowLayout {
        Layout.fillWidth:true
        Item { Layout.fillWidth:true }
        SButton { text:root.linked?'Proposals':'Linked strategies'; onClicked:root.linked=!root.linked }
    }
    DocumentsView { visible:root.linked; kind:'strategy'; embedded:true; strategyTab:'Execution'; Layout.fillWidth:true; Layout.fillHeight:true }
    ColumnLayout {
        visible:!root.linked; Layout.fillWidth:true; Layout.fillHeight:true; spacing:10
        Choice { id:statusChoice; model:['all','ready','deferred','applying','applied','rejected']; currentIndex:model.indexOf(root.status); onActivated:{root.status=model[currentIndex];Backend.refreshProposals(0,root.status);} Accessible.name:'Proposal status filter' }
        ListView {
            id:list; Layout.fillWidth:true; Layout.fillHeight:true; clip:true; spacing:1
            model:Backend.proposals
            ScrollBar.vertical:SScrollBar {}
            delegate:Rectangle {
                id:card; required property var modelData; width:list.width;height:body.implicitHeight+12;color:Theme.panel;radius: Theme.cornerRadius
                ColumnLayout {
                    id:body;anchors.left:parent.left;anchors.right:parent.right;anchors.top:parent.top;anchors.margins:6;spacing:3
                    RowLayout {
                        Layout.fillWidth:true
                        LabelText { text:card.modelData.title; textFormat:Text.PlainText; Layout.fillWidth:true; elide:Text.ElideRight }
                        LabelText { text:card.modelData.status; color:card.modelData.status==='applied'?Theme.green:Theme.accent; font.pixelSize:12 }
                    }
                    LabelText { text:card.modelData.source_title+' · '+(card.modelData.date || 'Unscheduled')+(card.modelData.schedule?' '+card.modelData.schedule.time:''); textFormat:Text.PlainText;color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap;font.pixelSize:12 }
                    RowLayout {
                        SButton { text:'Source';quiet:true;onClicked:Backend.showHistory(card.modelData.source_id) }
                        Item { Layout.fillWidth:true }
                        SButton { text:card.modelData.status==='applied'?'Inspect result':'Review';onClicked:Backend.reviewProposalRequested(card.modelData.id) }
                    }
                }
            }
            LabelText { anchors.centerIn:parent;visible:list.count===0 && !Backend.proposalLoading;text:'No proposals';color:Theme.muted }
        }
        RowLayout {
            Layout.fillWidth:true
            LabelText { text:Backend.proposalTotal ? ('Showing '+(Backend.proposalOffset+1)+'–'+Math.min(Backend.proposalOffset+Backend.proposals.length,Backend.proposalTotal)+' of '+Backend.proposalTotal) : ''; color:Theme.muted; Layout.fillWidth:true }
            SButton { text:'Previous'; quiet:true; enabled:!Backend.proposalLoading && Backend.proposalOffset>0; onClicked:Backend.refreshProposals(Math.max(0,Backend.proposalOffset-Backend.proposalLimit),root.status) }
            SButton { text:'Next'; quiet:true; enabled:!Backend.proposalLoading && Backend.proposalOffset+Backend.proposals.length<Backend.proposalTotal; onClicked:Backend.refreshProposals(Backend.proposalOffset+Backend.proposalLimit,root.status) }
        }
    }
}
