import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import QtQuick.Dialogs

Item {
    id: root
    property var sources: []
    property var analysis: ({})
    property bool busy: false
    property string error: ''
    property string activeTab: ['Feed', 'YouTube Intel', 'Notes', 'Sources'].indexOf(Backend.view) >= 0 ? Backend.view : 'Feed'
    property bool studyNotes: root.activeTab === 'Notes'
    property bool collectors: false
    property bool discoveries: root.activeTab === 'Feed'
    property string pendingSourceId: ""
    property int sourceOffset: 0
    property int sourceTotal: 0
    property string sourceQuery: ''
    property string policyGoalId: ''
    property var policy: ({})
    property var briefing: ({})
    property string briefingJobId: ''
    property bool briefingBusy: false
    property string briefingError: ''
    property bool briefingStale: false
    property int selectionEpoch: 0
    property bool briefingListing: false
    property bool alive: true
    property var briefingProgress: ({})
    property string briefingState: ''
    property var briefingChunks: []
    property var chunkEvidence: ({})
    property var citation: ({})
    property bool proposalBusy: false
    property int proposalRequest: 0
    function reload() {
        Backend.send('history-list',{kind:'all',q:root.sourceQuery,offset:root.sourceOffset,limit:50},(r,ok,e)=>{if(!root || !root.alive)return;if(ok){sources=r.items || [];root.sourceTotal=r.total || 0;if(root.pendingSourceId){let n=sources.findIndex(v=>v.id===root.pendingSourceId);if(n>=0){source.currentIndex=n;root.pendingSourceId='';}}if(source.currentIndex>=0)root.resetForSource();}else error=e || 'Could not read sources';});
    }
    function selectedSource() {
        return source.currentIndex >= 0 && source.currentIndex < root.sources.length ? root.sources[source.currentIndex] : null;
    }
    function fileUrlPath(value) {
        let text = String(value || '');
        if (text.indexOf('file://') === 0)
            text = text.slice(7);
        try { return decodeURIComponent(text); } catch (e) { return text; }
    }
    function resetForSource() {
        root.selectionEpoch += 1;
        root.busy = false;
        root.error = "";
        briefingTimer.stop();
        root.analysis = {};
        root.policy = {};
        root.briefing = {};
        root.briefingError = '';
        root.briefingStale = false;
        root.briefingBusy = false;
        root.briefingJobId = '';
        root.proposalRequest += 1;
        root.proposalBusy = false;
        root.loadLatestBriefing();
        root.evaluatePolicy();
    }
    function evaluatePolicy(done) {
        let current = root.selectedSource();
        if (!current) {
            if (done) done(null, false);
            return ;
        }
        let epoch = root.selectionEpoch, sourceId = String(current.id || '');
        let refs = current.source_refs || [];
        let url = current.url || (refs.length ? refs[0].url || '' : '');
        // A delayed policy response must not replace the state for a new source.
        let callback = (r,ok,e) => {
            if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId)
                return;
            if(ok){root.policy=r;if(done)done(r,true);}else{root.error=e || 'Could not evaluate source preference';if(done)done(null,false);}
        };
        Backend.send('source-policy-evaluate',{source_id:current.id,url:url,title:current.title,goal_id:root.policyGoalId},callback);
    }
    function setPolicy(kind, goal) {
        let current = root.selectedSource();
        if (!current)
            return ;
        if (goal && !root.policyGoalId.trim()) {
            root.error = 'Choose a goal for this source preference';
            return ;
        }
        Backend.send('source-policy-set',{scope:goal?'goal':'global',goal_id:goal?root.policyGoalId:'',kind:kind,matcher:'source',pattern:current.id},(r,ok,e)=>{if(!root || !root.alive)return;if(ok){root.error='';root.policy=r;evaluatePolicy();}else root.error=e || 'Could not save source preference';});
    }
    function briefingSegmentIds() {
        var seen = ({}), ids = [];
        (root.analysis.items || []).forEach(function(item) {
            var id = String(item.segment_id || '');
            if (id && !seen[id] && ids.length < 200) { seen[id] = true; ids.push(id); }
        });
        return ids;
    }
    function validBriefing(value, sourceId) {
        return value && value.schema === 'soliton-briefing/1' && value.source_id === sourceId && Array.isArray(value.candidate_actions);
    }
    function reviewBriefingProposal(action) {
        var ids = action && Array.isArray(action.segment_ids) ? action.segment_ids : [];
        var sourceId = String(root.briefing.source_id || '');
        var pinnedRevision = String(root.briefing.source_revision || '');
        if (!sourceId || !pinnedRevision || !ids.length) {
            root.briefingError = 'This briefing does not contain a complete pinned source citation.';
            return;
        }
        if (root.briefingStale) {
            root.briefingError = 'This briefing is stale. Re-run it before reviewing a proposal.';
            return;
        }
        if (Backend.proposalDraft && Backend.proposalDraft.source_id && Backend.proposalDraft.title) {
            root.briefingError = 'Finish or discard the existing proposal draft before reviewing another action.';
            return;
        }
        var token = ++root.proposalRequest;
        var segmentId = String(ids[0] || '');
        root.proposalBusy = true;
        root.briefingError = '';
        Backend.send('history-detail', {id:sourceId, revision:pinnedRevision}, (record, ok, error) => {
            if (!root || !root.alive || token !== root.proposalRequest)
                return;
            root.proposalBusy = false;
            if (!ok) {
                root.briefingError = error || 'The pinned source revision could not be loaded. Review was not opened.';
                return;
            }
            if (!record || String(record.id || '') !== sourceId || String(record.revision || '') !== pinnedRevision) {
                root.briefingError = 'The cited source revision changed or is unavailable. Review the current source before proposing work.';
                return;
            }
            var segment = (record.segments || []).find(item => String(item.id || '') === segmentId);
            if (!segment || !String(segment.text || '').trim()) {
                root.briefingError = 'The cited passage is no longer available in the pinned revision. Review the source before proposing work.';
                return;
            }
            Backend.beginProposal({id:record.id, revision:record.revision, title:record.title || root.briefing.title || sourceId}, {id:segment.id, text:segment.text});
            var draft = Object.assign({}, Backend.proposalDraft);
            var suggestedTitle = String(action.title || '').trim();
            var suggestedRationale = String(action.rationale || '').trim();
            if (suggestedTitle)
                draft.title = suggestedTitle.slice(0, 500);
            if (suggestedRationale)
                draft.rationale = suggestedRationale.slice(0, 8000);
            if(action.goal_id&&(Backend.goals||[]).some(g=>g.id===action.goal_id))draft.goal_id=action.goal_id;
            Backend.proposalDraft = draft;
            Backend.remember();
        });
    }
    function openCitation(segmentId) {
        let sourceId=root.briefing.source_id, revision=root.briefing.source_revision;
        Backend.send('history-detail',{id:sourceId,revision:revision},(r,ok,e)=>{
            if(!root||!root.alive)return;
            if(!ok){root.briefingError=e;return;}
            let segment=(r.segments||[]).find(s=>s.id===segmentId);
            if(!segment){root.briefingError='The cited passage is missing from the pinned source revision.';return;}
            root.citation={title:r.title,source_id:sourceId,revision:revision,segment:segment,url:segment.url||((r.source_refs||[])[0]||{}).url||''};
            citationDialog.open();
        });
    }
    function loadLatestBriefing() {
        let current = root.selectedSource();
        if (!current) return;
        let epoch = root.selectionEpoch, sourceId = String(current.id || '');
        root.briefingListing = true;
        Backend.send('briefing-list',{source_id:sourceId,limit:20},(r,ok,e)=>{
            if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
            root.briefingListing = false;
            if (!ok) { root.briefingError=e || 'Could not recover local briefing jobs'; return; }
            let items = r.items || [];
            if (!items.length) return;
            root.briefingJobId = String(items[0].job_id || '');
            root.briefingBusy = items[0].state === 'queued' || items[0].state === 'running' || items[0].state === 'cancel_requested';
            if(root.briefingBusy)briefingTimer.start();
            root.pollBriefing(epoch, sourceId, root.briefingJobId);
        });
    }
    function pollBriefing(expectedEpoch, expectedSourceId, expectedJobId) {
        if (!root.briefingJobId) return;
        let epoch = expectedEpoch === undefined ? root.selectionEpoch : expectedEpoch;
        let sourceId = expectedSourceId || (root.selectedSource() ? String(root.selectedSource().id || '') : '');
        let jobId = expectedJobId || root.briefingJobId;
        Backend.send('briefing-status',{job_id:jobId},(r,ok,e)=>{
            if (!root || !root.alive || epoch !== root.selectionEpoch || root.briefingJobId !== jobId || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
            if (!ok) { root.briefingBusy=false; root.briefingError=e || 'Could not read local briefing status'; briefingTimer.stop(); return; }
            root.briefingStale = !!r.stale;
            root.briefingProgress=r.coverage||{};root.briefingState=r.state;root.briefingChunks=r.chunks||[];
            if (r.state === 'queued' || r.state === 'running' || r.state === 'cancel_requested') {root.briefingBusy=true;if(!briefingTimer.running)briefingTimer.start();return;}
            briefingTimer.stop(); root.briefingBusy=false;
            if (r.state === 'complete' && root.validBriefing(r.result, root.selectedSource() ? root.selectedSource().id : '')) {
                root.briefing = r.result; root.briefingError='';
            } else if (r.state === 'waiting') {
                root.briefingError = '';
            } else if (r.state === 'failed') {
                root.briefingError = r.error || 'The local model did not produce a briefing.';
            } else if (r.state === 'paused') {
                root.briefingError = r.error || 'Paused after restart; resume to run the local model.';
            } else if (r.state === 'cancel_requested' || r.state === 'cancelled') {
                root.briefingError = r.state === 'cancel_requested' ? 'Cancellation requested; waiting for the local worker receipt.' : 'Local briefing cancelled before completion.';
            } else {
                root.briefingError = 'The local model returned an unrecognized briefing shape.';
            }
        });
    }
    function startBriefing() {
        var current = root.selectedSource();
        if (!current || root.briefingBusy) return;
        let epoch = root.selectionEpoch, sourceId = String(current.id || '');
        root.briefingBusy=true; root.briefingError=''; root.briefing={}; root.briefingStale=false;
        root.evaluatePolicy(function(policy, policyOk) {
            if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
            if (!policyOk) { root.briefingBusy=false; root.briefingError='Could not evaluate the source preference.'; return; }
            if (policy.include === false) { root.briefingBusy=false; root.briefingError='This source is excluded by a local preference. Review the preference before generating a briefing.'; return; }
            var goals = root.policyGoalId.trim() ? [root.policyGoalId.trim()] : [];
            Backend.send('briefing-start',{source_id:current.id,goal_ids:goals},(r,ok,e)=>{
                if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
                if (!ok) { root.briefingBusy=false; root.briefingError=e || 'Could not queue local briefing'; return; }
                if (!r.job_id) { root.briefingBusy=false; root.briefingError='The local briefing queue returned no job receipt.'; return; }
                root.briefingJobId=r.job_id; briefingTimer.start(); root.pollBriefing(epoch, sourceId, r.job_id);
            });
        });
    }
    function analyzeCurrent() {
        let current = root.selectedSource();
        if (!current || root.busy) return;
        let epoch = root.selectionEpoch, sourceId = String(current.id || '');
        root.evaluatePolicy(function(policy, policyOk) {
            if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
            if (!policyOk) return;
            if (policy.include === false) { root.error='This source is excluded by a local preference. Review it read-only or change the preference.'; return; }
            root.busy=true; root.error='';
            Backend.send('intelligence-analyze',{source_id:sourceId,goal_id:root.policyGoalId},(r,ok,e)=>{
                if (!root || !root.alive || epoch !== root.selectionEpoch || !root.selectedSource() || String(root.selectedSource().id || '') !== sourceId) return;
                root.busy=false;
                if(ok)root.analysis=r;else root.error=e || 'Analysis failed';
            });
        });
    }
    Component.onCompleted:{reload();Backend.refreshModelChosen()}
    // Sources reads afresh whenever it is opened, from the side panel or the palette as from its former tab.
    onActiveTabChanged: if (activeTab === 'Sources') reload()
    Component.onDestruction:{root.alive=false;briefingTimer.stop()}
    // Popups and helpers sit outside the layout, so it never manages them.
    ColumnLayout {
        anchors.fill: parent
        spacing: 12
        // Under the side panel treatment the views are listed there; the strip
        // and its row then take no height, so each view starts at the page top.
        RowLayout {
            visible: Theme.pageTabs
            Layout.fillWidth: true
            Layout.fillHeight: false
            spacing: Theme.space8

            STabBar {
                Layout.fillWidth: true

                Repeater {
                    model: ['Feed', 'YouTube Intel', 'Notes', 'Sources']
                    delegate: STabButton {
                        required property string modelData
                        text: modelData
                        active: root.activeTab === modelData
                        onClicked: {
                            Backend.navigate('Learning', modelData)
                            if (modelData === 'Sources')
                                root.reload()
                        }
                    }
                }
                Item { Layout.fillWidth: true }
            }

        }

        LearningFeed {
            visible: root.activeTab === 'Feed'
            Layout.fillWidth: true
            Layout.fillHeight: true
            onArchiveRequested: function(id) {
                root.pendingSourceId = id
                root.sourceQuery = ""
                root.sourceOffset = 0
                Backend.navigate('Learning', 'Sources')
                root.reload()
            }
            onVideosRequested: function(view) {
                youtube.activeView = view
                Backend.navigate('Learning', 'YouTube Intel')
            }
        }

        YouTubeIntelView {
            id: youtube
            visible: root.activeTab === 'YouTube Intel'
            Layout.fillWidth: true
            Layout.fillHeight: true
        }

        DocumentsView {
            visible: root.activeTab === 'Notes'
            Layout.fillWidth: true
            Layout.fillHeight: true
            kind: 'learning'
        }

        ColumnLayout {
            visible: root.activeTab === 'Sources'
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 12
            SButton { text:'Source settings'; quiet:true; onClicked:sourceSettings.open() }
            RowLayout {
                Layout.fillWidth:true
                Field { id:path; Layout.fillWidth:true; placeholderText:'Absolute path to a transcript, article or notes file'; Accessible.name:'Source file path' }
                SButton { text:'Browse'; quiet:true; onClicked:sourcePicker.open() }
                SButton {
                    text:'Import source'; enabled:!root.busy && path.text.trim().length>0
                    onClicked:{root.busy=true;root.error='';Backend.send('history-import',{path:path.text,provider:'manual'},(r,ok,e)=>{if(!root || !root.alive)return;root.busy=false;if(ok){path.text='';root.reload();}else root.error=e || 'Source import failed';});}
                }
            }
            RowLayout {
                Layout.fillWidth:true
                SSearchField { Layout.fillWidth:true; scopeName:'archived sources'; text:root.sourceQuery; onTextEdited:root.sourceQuery=text; onAccepted:{root.sourceOffset=0;root.reload();} }
                SButton { text:'Find'; quiet:true; onClicked:{root.sourceOffset=0;root.reload();} }
                SButton { text:'‹'; quiet:true; enabled:root.sourceOffset>0; onClicked:{root.sourceOffset=Math.max(0,root.sourceOffset-50);root.reload();} }
                SButton { text:'›'; quiet:true; enabled:root.sourceOffset+root.sources.length<root.sourceTotal; onClicked:{root.sourceOffset+=50;root.reload();} }
            }
            RowLayout {
                Layout.fillWidth:true
                Choice { id:source; Layout.fillWidth:true; model:root.sources; textRole:'title'; onCurrentIndexChanged:root.resetForSource(); Accessible.name:'Archived source' }
                Choice { id:goalChoice; Layout.preferredWidth:220; model:[{id:'',title:'All goals'}].concat(Backend.goals || []); textRole:'title'; onActivated:{root.policyGoalId=String(model[currentIndex].id || '');root.resetForSource();} Accessible.name:'Goal focus' }
            }
            Flow {
                Layout.fillWidth:true; spacing:8
                // The analysis and briefing controls exist only with the Local model setting on.
                Loader {
                    active: Backend.prefs.localModel === true
                    visible: active
                    sourceComponent: SButton {
                        busy: root.busy

                        text:root.busy?'Reading…':'Find useful passages'; primary:true; enabled:!root.busy && source.currentIndex>=0 && root.sources.length>0
                        onClicked:root.analyzeCurrent()
                    }
                }
                Loader {
                    active: Backend.prefs.localModel === true
                    visible: active
                    sourceComponent: SButton {
                        busy: root.briefingBusy
         text:!Backend.modelChosen?'Waiting for your model':root.briefingBusy?'Generating local briefing…':'Generate local briefing'; primary:true; enabled:Backend.modelChosen && !root.busy && !root.briefingBusy && source.currentIndex>=0 && !!root.selectedSource(); onClicked:root.startBriefing() }
                }
                SButton { visible:root.briefingBusy && !!root.briefingJobId; text:'Cancel briefing'; quiet:true; enabled:!root.busy; onClicked:{let jobId=root.briefingJobId;Backend.send('briefing-cancel',{job_id:jobId},(r,ok,e)=>{if(!root || !root.alive || root.briefingJobId!==jobId)return;if(ok)root.briefingError='Cancellation requested; waiting for the local worker receipt.';else root.briefingError=e || 'Could not request briefing cancellation';})} }
                Loader {active:Backend.prefs.localModel === true&&Backend.modelChosen&&['paused','failed','cancelled'].indexOf(root.briefingState)>=0&&!root.briefingStale;visible:active;sourceComponent:SButton {text:'Resume unfinished sections';enabled:!root.briefingBusy;onClicked:Backend.send('briefing-resume',{job_id:root.briefingJobId},(r,ok,e)=>{if(!root||!root.alive)return;if(ok){root.briefingBusy=true;root.briefingError='';briefingTimer.start();root.pollBriefing();}else root.briefingError=e;})}}
                SButton {text:'Inspect sections';visible:root.briefingChunks.length>1;onClicked:sectionsDialog.open()}
                SButton { text:'Source history'; onClicked:{Backend.strategyTab='History';Backend.navigate('Strategy','History');} }
            }
            RowLayout {
                Layout.fillWidth:true
                LabelText { visible:false; text:root.policyGoalId }
                SButton { text:'Use source'; quiet:true; enabled:source.currentIndex>=0; onClicked:root.setPolicy('save',false) }
                SButton { text:'Exclude source'; quiet:true; enabled:source.currentIndex>=0; onClicked:root.setPolicy('exclude',false) }
                SButton { text:'Use for goal'; quiet:true; enabled:source.currentIndex>=0 && !!root.policyGoalId; onClicked:root.setPolicy('save',true) }
                SButton { text:'Exclude for goal'; quiet:true; enabled:source.currentIndex>=0 && !!root.policyGoalId; onClicked:root.setPolicy('exclude',true) }
                LabelText { Layout.fillWidth:true; color:root.policy.excluded ? Theme.danger : Theme.muted; text:root.policy.reason || 'No source preference evaluated'; elide:Text.ElideRight }
            }
            NoticePopup { scope: root; text: root.error; onDismissed: root.error = '' }
            NoticePopup { scope: root; text: root.analysis.notice || ''; error: false }
            LabelText { visible:!!root.analysis.source_id; text:(root.analysis.segments_examined || 0)+' / '+(root.analysis.segments_total || 0)+' passages examined · '+(root.analysis.matches_total || 0)+' candidates'+(root.analysis.limited?' · Showing a bounded selection':''); color:Theme.accent; font.pixelSize:12 }
            Flickable {
                ScrollBar.vertical:SScrollBar {}
                visible:!!root.briefing.schema; Layout.fillWidth:true; Layout.fillHeight:!root.analysis.source_id; Layout.minimumHeight:200;Layout.preferredHeight:Math.min(400, briefingCard.implicitHeight); contentWidth:width; contentHeight:briefingCard.implicitHeight; clip:true
                Rectangle {
                id:briefingCard; width:parent.width; color:Theme.panel; border.color:root.briefingStale ? Theme.danger : Theme.accent; radius: Theme.cornerRadius; implicitHeight:briefingBody.implicitHeight+28
                ColumnLayout { id:briefingBody; anchors.fill:parent; anchors.margins:14; spacing:8
                    LabelText { text:'Local briefing'; font.pixelSize:18; font.weight:Font.DemiBold }
                    LabelText { Layout.fillWidth:true; text:root.briefing.summary || ''; font.pixelSize: Theme.readerSize; textFormat:Text.PlainText; wrapMode:Text.Wrap }
                    LabelText { Layout.fillWidth:true; visible:root.briefingStale; text:'The source changed after this briefing was queued. Re-run it before using any proposal.'; color:Theme.dangerText; wrapMode:Text.Wrap }
                    LabelText { Layout.fillWidth:true; text:(root.briefing.candidate_actions || []).length+' candidate actions · '+((root.briefing.coverage || {}).segments_evaluated || 0)+' archived passages evaluated · exact source citations'; color:Theme.muted; font.pixelSize:12; wrapMode:Text.Wrap }
                    LabelText {visible:!!(root.briefing.coverage||{}).actions_truncated;text:'The earlier processor limited this result. Reprocess to obtain a complete synthesis.';color:Theme.dangerText;Layout.fillWidth:true;wrapMode:Text.Wrap}
                    Repeater {
                        model:root.briefing.candidate_actions || []
                        delegate:Rectangle { required property var modelData; Layout.fillWidth:true; implicitHeight:briefingAction.implicitHeight+16; color:Theme.base; border.color:Theme.line; radius: Theme.cornerRadius
                            ColumnLayout { id:briefingAction; anchors.fill:parent; anchors.margins:8; spacing:4
                                LabelText { Layout.fillWidth:true; text:modelData.title || ''; textFormat:Text.PlainText; font.weight:Font.DemiBold; wrapMode:Text.Wrap }
                                LabelText { Layout.fillWidth:true; text:modelData.rationale || ''; font.pixelSize: 14; textFormat:Text.PlainText; color:Theme.muted; wrapMode:Text.Wrap }
                                LabelText { Layout.fillWidth:true; text:((modelData.segment_ids || []).length===1?'Cites 1 passage':'Cites '+(modelData.segment_ids || []).length+' passages')+(modelData.goal_id ? ' · Goal: '+(((Backend.goals || []).find(g=>g.id===modelData.goal_id) || {}).title || 'a goal no longer listed') : ''); textFormat:Text.PlainText; color:Theme.accent; font.pixelSize: 12; wrapMode:Text.Wrap }
                                Flow { Layout.fillWidth:true; spacing:6
                                    Repeater { model:modelData.segment_ids || []; delegate:SButton { required property string modelData; required property int index; text:'Passage '+(index+1); quiet:true; onClicked:root.openCitation(modelData) } }
                                    SButton {
                    busy: root.proposalBusy
     text:root.proposalBusy?'Loading citation…':'Review proposal'; quiet:true; enabled:!root.proposalBusy && (modelData.segment_ids || []).length>0; onClicked:root.reviewBriefingProposal(modelData) }
                                }
                            }
                        }
                    }
                }
            }
            }
            LabelText {visible:root.briefingBusy;text:'Processing '+(root.briefingProgress.chunks_complete||0)+' / '+(root.briefingProgress.chunks_total||1)+' sections · '+(root.briefingProgress.selected_segments||root.briefingProgress.source_segments||0)+' source passages';color:Theme.accent;Layout.fillWidth:true;wrapMode:Text.Wrap}
            LabelText { visible:!!root.briefingError; text:root.briefingError; color:Theme.dangerText; Layout.fillWidth:true; wrapMode:Text.Wrap }
            LabelText { visible:root.briefingState === 'waiting' && !root.briefingError; text:'Waiting for your model'; color:Theme.muted; Layout.fillWidth:true }
            ListView {
                id:list; visible:!root.briefing.schema||!!root.analysis.source_id; Layout.fillWidth:true; Layout.fillHeight:true; model:root.analysis.items || []; clip:true; spacing:10
                ScrollBar.vertical:SScrollBar {}
                delegate:Rectangle {
                    id:passage; required property var modelData
                    width:list.width; height:content.implicitHeight+28; radius: Theme.cornerRadius; color:Theme.panel; border.color:Theme.line
                    ColumnLayout {
                        id:content; anchors.left:parent.left;anchors.right:parent.right;anchors.top:parent.top;anchors.margins:14;spacing:9
                        LabelText { text:passage.modelData.text; textFormat:Text.PlainText; wrapMode:Text.Wrap; Layout.fillWidth:true; font.pixelSize:14 }
                        LabelText { text:(passage.modelData.goals || []).map(g=>g.title+' — '+g.reason).join('\n') || 'No goal match found. Choose a goal yourself if this is relevant.'; textFormat:Text.PlainText; color:Theme.muted; font.pixelSize:12; wrapMode:Text.Wrap; Layout.fillWidth:true }
                        RowLayout {
                            SButton { text:'Read source'; quiet:true; onClicked:Backend.showHistory(root.analysis.source_id) }
                            Item { Layout.fillWidth:true }
                            SButton { text:'Propose work'; onClicked:Backend.beginProposal({id:root.analysis.source_id,revision:root.analysis.source_revision,title:root.analysis.title},{id:passage.modelData.segment_id,text:passage.modelData.text}) }
                        }
                    }
                }
                LabelText { anchors.centerIn:parent; visible:list.count===0 && !root.busy; text:root.analysis.source_id?'No action-language or goal-term matches. Read the source to make your own proposal.':'Choose an archived source, or import a text file, to begin.'; color:Theme.muted; width:Math.min(parent.width,560); wrapMode:Text.Wrap; horizontalAlignment:Text.AlignHCenter }
            }
        }
    }

    SPopup {
        id:sectionsDialog;anchors.centerIn:parent;width:Math.min(820,root.width-24);height:Math.min(650,root.height-24);modal:true;focus:true;padding:22
        background:Rectangle {color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius}
        ColumnLayout {anchors.fill:parent;spacing:12
            LabelText {text:'Source coverage';font.pixelSize:23}
            LabelText {text:'Every completed section retains its own analysis and source citations. The final briefing consolidates these into a shorter decision list.';color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap}
            Flow {Layout.fillWidth:true;Layout.preferredHeight:implicitHeight;spacing:6
                Repeater {model:root.briefingChunks;delegate:SButton {required property var modelData;text:'Section '+(modelData.index+1)+' · '+modelData.state;enabled:modelData.state==='complete';onClicked:Backend.send('briefing-chunk',{job_id:root.briefingJobId,index:modelData.index},(r,ok,e)=>{if(!root||!root.alive)return;if(ok)root.chunkEvidence=r;else root.briefingError=e;})}}
            }
            ScrollView { contentWidth: availableWidth;Layout.fillWidth:true;Layout.fillHeight:true
                STextArea {text:(root.chunkEvidence.result||{}).summary||'Choose a completed section to read its retained analysis.';readOnly:true;selectByMouse:true;wrapMode:TextEdit.Wrap;color:Theme.text;background:null}
            }
            SButton {text:'Close';Layout.alignment:Qt.AlignRight;onClicked:sectionsDialog.close()}
        }
    }
    SPopup {
        id:citationDialog;anchors.centerIn:parent;width:Math.min(760,root.width-24);height:Math.min(620,root.height-24);modal:true;focus:true;padding:22
        background:Rectangle {color:Theme.panel;border.color:Theme.line;radius: Theme.cornerRadius}
        ColumnLayout {anchors.fill:parent;spacing:12
            LabelText {text:root.citation.title||'Source evidence';font.pixelSize:23;Layout.fillWidth:true;wrapMode:Text.Wrap}
            LabelText {text:'Revision '+(root.citation.revision||'');color:Theme.accent;Layout.fillWidth:true;wrapMode:Text.Wrap}
            ScrollView { contentWidth: availableWidth;Layout.fillWidth:true;Layout.fillHeight:true;STextArea {text:(root.citation.segment||{}).text||'';readOnly:true;selectByMouse:true;wrapMode:TextEdit.Wrap;color:Theme.text;font.pixelSize:15;background:null}}
            RowLayout {Layout.alignment:Qt.AlignRight
                SButton {text:'Open original';visible:!!root.citation.url;onClicked:Qt.openUrlExternally(root.citation.url)}
                SButton {text:'Close';primary:true;onClicked:citationDialog.close()}
            }
        }
    }
    SPopup {
        id: sourceSettings
        parent: Overlay.overlay
        anchors.centerIn: parent
        width: Math.min(940, parent.width - 24)
        height: Math.min(720, parent.height - 24)
        modal: true
        focus: true
        padding: 12
        background: Rectangle { color:Theme.base; border.color:Theme.line; radius:Theme.cornerRadius }
        contentItem: ColumnLayout {
            CollectionView { Layout.fillWidth:true; Layout.fillHeight:true; visible:sourceSettings.visible }
            SButton { text:'Close'; Layout.alignment:Qt.AlignRight; onClicked:sourceSettings.close() }
        }
        onClosed: root.reload()
    }
    FileDialog { id:sourcePicker; title:'Choose a source file'; fileMode:FileDialog.OpenFile; onAccepted:path.text=root.fileUrlPath(selectedFile) }
    Timer { id:briefingTimer; interval:700; repeat:true; onTriggered:root.pollBriefing() }
}
