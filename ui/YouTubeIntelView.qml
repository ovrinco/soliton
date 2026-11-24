pragma ComponentBehavior: Bound
import QtQuick
import QtQuick.Controls
import QtQuick.Layouts
import "TranscriptDocument.js" as TranscriptDocument
import "Workplane.js" as Workplane

Item {
    id: root
    property string activeView: 'Suggestions'
    property string intakeUrl: ''
    property string query: ''
    property string errorText: ''
    property string noticeText: ''
    property bool busy: false
    property var discoveries: []
    property var suggestion: null
    property var configured: []
    property var jobs: []
    property var resultRecord: null
    property var resultBriefing: null
    property string selectedJobId: ''
    property string selectedResultId: ''
    property string resultView: 'Transcript'
    property string analysisState: ''
    property bool loading: false
    property var archiveRows: []
    property var briefingRows: []
    property int briefingRefreshAt: 0
    property var citationIds: []
    property string analysisJob: ''
    // A persisted transcript is still work to analyze. Only the briefing
    // owner's completed analysis receipt can place a source in Results.
    property var resultRows: briefingRows.filter(row => row.state === 'complete' && row.source_id &&
        (root.archiveRows.some(record => record.id === row.source_id) || root.jobs.some(job => (job.history_ids || []).indexOf(row.source_id) >= 0))).map(function(row) {
        return {id:'analysis:'+row.job_id, title:row.title || 'YouTube analysis', history_ids:[row.source_id], state:'complete', coverage:row.coverage || {}}
    })
    property var intakeRows: jobs.filter(job => !root.briefingRows.some(row => row.state === 'complete' && (job.history_ids || []).indexOf(row.source_id) >= 0)).map(function(job) {
        var source = root.configured.find(item => item.id === job.source_id) || {}
        return {id:'intake:'+job.id, title:job.result && job.result.title || source.title || source.location || 'YouTube source', publisher:'YouTube', url:source.location || '', provider:'youtube', collectionState:job.state, error:job.error || '', history_ids:job.history_ids || []}
    }).concat(root.archiveRows.filter(record => !root.briefingRows.some(row => row.state === 'complete' && row.source_id === record.id) && !root.jobs.some(job => (job.history_ids || []).indexOf(record.id) >= 0)).map(record => ({id:'archive:'+record.id,title:record.title,publisher:record.channel||'YouTube',url:record.url||'',history_ids:[record.id],collectionState:'completed'})))
    // The YouTube suggester's picks: their own view, not mixed with collected videos.
    property var suggestionRows: discoveries.filter(item => !root.jobs.some(job => job.source_id === item.id) && (!root.query || (String(item.title || '') + ' ' + String(item.publisher || '')).toLowerCase().indexOf(root.query.toLowerCase()) >= 0))
    property bool intakeSelected: intakeRows.some(row => String(row.id || '') === selectedJobId)

    function loadSources() {
        if (loading || !visible) return;
        loading = true;
        Backend.send('collection-discover', {}, function(value, ok, error) {
            if (!root) return
            if (!ok) { root.errorText = error || 'YouTube suggestions are unavailable.'; return }
            root.discoveries = (value.items || []).filter(item => root.validUrl(item.url || ''))
            if (!root.suggestion && root.discoveries.length) root.suggestion = root.discoveries[0]
        }, true)
        Backend.send('collection-list', {limit: 100}, function(value, ok, error) {
            if (!root) return
            root.loading = false
            if (!ok) { root.errorText = error || 'Saved video sources are unavailable.'; return }
            root.configured = (value.items || []).filter(item => item.kind === 'youtube-video')
            root.jobs = (value.jobs || []).filter(job => root.configured.some(source => source.id === job.source_id))
            root.retainJobSelection()
            var selected = root.jobs.find(job => String(job.id) === root.selectedJobId)
            if (selected && !root.resultRecord && (selected.history_ids || []).length) root.openResult(String(selected.history_ids[0]))
        }, true)
        Backend.send('history-list', {kind:'source', provider:'youtube', limit:100}, function(value, ok, error) {
            if (!root) return;
            if (ok) root.archiveRows = (value.items || []).filter(record => record.provider === 'youtube' || root.validUrl(record.url || ''));
            else root.errorText = error;
        }, true)
        if (Date.now() - root.briefingRefreshAt >= 5000) {
            root.briefingRefreshAt = Date.now()
            Backend.send('briefing-list', {limit:50}, function(value, ok, error) {
                if (!root) return
                if (ok) { root.briefingRows = (value.items || []).filter(row => row.source_id); root.retainJobSelection() }
                else root.errorText = error || 'Analysis history could not be read.'
            }, true)
        }
    }

    function retainJobSelection() {
        if (!root.selectedJobId && root.resultRows.length)
            root.selectedJobId = String(root.resultRows[0].id || '')
        if (root.selectedJobId && !root.resultRows.concat(root.intakeRows).some(function(job) { return String(job.id || '') === root.selectedJobId }))
            root.selectedJobId = ''
    }

    function validUrl(value) {
        return /^https?:\/\/(www\.|m\.)?(youtube\.com\/watch\?v=[A-Za-z0-9_-]{11}|youtu\.be\/[A-Za-z0-9_-]{11})(?:[&#?].*)?$/.test(value)
    }
    function coverageLabel(coverage) {
        if (!coverage) return ''
        var total = Number(coverage.source_segments || 0)
        var selected = Number(coverage.selected_segments || 0)
        return total && selected && selected < total ? ' · partial ' + selected + '/' + total + ' segments' : ''
    }
    // Plain words for a collection job's state; a transcript is claimed only by
    // the Read transcript action, which appears when the record exists.
    function collectionLabel(state) {
        return {queued:'Waiting to collect', running:'Collecting', completed:'Collected', failed:'Collection failed', cancelled:'Collection cancelled'}[state] || ''
    }
    // An analysis is called complete only when its result is loaded and opens.
    function analysisLabel(state) {
        if (state === 'complete') return root.resultBriefing ? 'Analysis complete' : 'Analysis finished without a readable result'
        if (state === 'waiting' || (state === 'queued' && !Backend.modelChosen)) return 'Waiting for your model'
        return {queued:'Analysis queued', running:'Analysing', cancel_requested:'Stopping analysis', cancelled:'Analysis cancelled', failed:'Analysis failed', paused:'Analysis paused', 'Source changed':'Source changed since the analysis'}[state] || ''
    }
    function shortNote(value, limit) {
        var text = Workplane.previewLine(value)
        return text.length > limit ? text.slice(0, limit - 1).replace(/\s+\S*$/, '') + '…' : text
    }
    function publishedLabel(value) {
        if (!value) return ''
        return typeof value === 'number' ? Qt.formatDate(new Date(value), 'MMM d, yyyy') : String(value)
    }

    function ingest(fromUrl) {
        var url = (typeof fromUrl === 'string' ? fromUrl : root.intakeUrl).trim()
        if (!validUrl(url)) { root.errorText = 'Enter one YouTube video URL.'; return }
        root.busy = true; root.errorText = ''; root.noticeText = ''
        Backend.send('collection-add', {type:'youtube-video', url:url, cadence_hours:24, auto_briefing:Backend.prefs.localModel === true, goal_ids:[]}, function(value, ok, error) {
            if (!root) return
            root.busy = false
            if (!ok) { root.errorText = error || 'The source was not admitted.'; return }
            root.intakeUrl = ''
            root.selectedJobId = String(value.job_id || '')
            root.noticeText = 'Queued'
            // Admission is not analysis. Keep the acknowledged operation in
            // the pre-analysis view until a persisted transcript and result
            // exist; a failure remains attached to this intake item.
            root.activeView = 'To analyze'
            root.loadSources()
            pollTimer.start()
        })
    }

    function selectJob(job) {
        if (!job) return
        root.selectedJobId = String(job.id || '')
        root.resultRecord = null
        root.resultBriefing = null
        root.analysisState = ''
        root.analysisJob = ''
        root.resultView = 'Transcript'
        root.errorText = ''
        var ids = job.history_ids || []
        if (ids.length) root.openResult(String(ids[0]))
        else root.noticeText = job.state === 'failed' ? '' : 'The persisted transcript is not available yet.'
    }

    function openResult(id) {
        if (root.selectedResultId !== id) { root.resultRecord = null; root.resultBriefing = null; root.analysisState = ''; root.analysisJob = ''; root.citationIds = []; root.resultView = 'Transcript'; }
        root.selectedResultId = id
        Backend.send('history-detail', {id:id}, function(value, ok, error) {
            if (!root || root.selectedResultId !== id) return
            if (!ok) { root.errorText = error || 'The persisted transcript could not be opened.'; return }
            root.resultRecord = value
            root.errorText = ''
            Backend.send('briefing-list', {source_id:id, limit:20}, function(items, listed, listingError) {
                if (!root || root.selectedResultId !== id) return
                if (!listed) { root.errorText = listingError || 'Analysis history could not be read.'; return }
                var rows = items.items || []
                var selectedAnalysisId = root.selectedJobId.startsWith('analysis:') ? root.selectedJobId.slice('analysis:'.length) : ''
                var selectedBriefing = selectedAnalysisId ? rows.find(row => row.job_id === selectedAnalysisId) : rows[0]
                if (!selectedBriefing) { root.analysisState = ''; return; }
                root.analysisJob = selectedBriefing.job_id;
                Backend.send('briefing-status', {job_id: selectedBriefing.job_id, source_revision: String(value.revision)}, function(job, ready, failure) {
                    if (!root || root.selectedResultId !== id) return;
                    if (!ready) { root.errorText = failure; return; }
                    root.analysisState = job.stale ? 'Source changed' : job.state;
                    if (job.state === 'complete' && job.result && job.result.summary) { root.resultBriefing = job.result; root.resultView = 'Analysis'; }
                    if (job.error) root.errorText = job.error;
                }, true)
            }, true)
        }, true)
    }

    function pollJobs() {
        Backend.send('collection-poll', {}, function(value, ok, error) {
            if (!root || !ok) { if (root && error) root.errorText = error; return }
            root.loadSources()
            if (root.resultRecord && ['queued','running','cancel_requested'].indexOf(root.analysisState) >= 0) root.openResult(root.selectedResultId)
        }, true)
    }
    function analyze() {
        if (!resultRecord || busy) return;
        busy = true; errorText = '';
        var source = selectedResultId;
        var goalIndex = root.activeView === 'To analyze' ? intakeGoal.currentIndex : goal.currentIndex;
        Backend.send('briefing-start', {source_id:source, goal_ids:goalIndex > 0 ? [Backend.goals[goalIndex-1].id] : []}, (result, ok, error) => {
            if (!root) return;
            busy = false;
            if (source !== selectedResultId) return;
            if (!ok) { errorText = error; return; }
            analysisJob = result.job_id; analysisState = result.state; openResult(source);
        });
    }

    Timer { id: pollTimer; interval:1800; repeat:true; running:root.visible && (root.jobs.some(job => job.state === 'queued' || job.state === 'running' || (job.state === 'completed' && !job.acked)) || ['queued','running','cancel_requested'].indexOf(root.analysisState) >= 0); onTriggered:root.pollJobs() }
    Component.onCompleted: if (visible) root.loadSources()
    onVisibleChanged: if (visible) { root.loadSources(); Backend.refreshModelChosen() }
    Component.onDestruction:pollTimer.stop()

    ColumnLayout {
        anchors.fill:parent; spacing:10
        RowLayout {
            Layout.fillWidth:true
            STabBar { Layout.fillWidth:true; Repeater { model:['Suggestions','To analyze','Results']; delegate:STabButton { required property string modelData; text:modelData; active:root.activeView===modelData; onClicked:root.activeView=modelData } } Item { Layout.fillWidth:true } }
        }
        RowLayout {
            visible:root.activeView==='To analyze'
            Layout.fillWidth:true
            Field { Layout.fillWidth:true; placeholderText:'YouTube video URL'; text:root.intakeUrl; onTextEdited:root.intakeUrl=text; onAccepted:root.ingest(); Accessible.name:'YouTube video URL' }
            SButton {
                busy: root.busy
 text:root.busy?'Adding…':'Add video'; primary:true; enabled:!root.busy&&root.validUrl(root.intakeUrl.trim()); onClicked:root.ingest() }
        }
        NoticePopup { scope: root; text: root.errorText || root.noticeText; error: !!root.errorText }


        SplitView {
            visible:root.activeView==='Suggestions'; Layout.fillWidth:true; Layout.fillHeight:true
            orientation:root.width < 750 ? Qt.Vertical : Qt.Horizontal
            handle:SSplitHandle { horizontal:root.width >= 750 }
            ColumnLayout {
                SplitView.preferredWidth:Math.max(420,root.width*.58); SplitView.minimumWidth:300; SplitView.preferredHeight:root.height*.5; SplitView.minimumHeight:160; spacing:0
                RowLayout {
                    Layout.fillWidth:true; spacing:8
                    SSearchField { Layout.fillWidth:true; frameless:true; scopeName:'suggestions'; text:root.query; onTextEdited:root.query=text }
                    LabelText { Layout.rightMargin:8; text:suggestionList.count+(suggestionList.count===1?' video':' videos'); color:Theme.muted; font.pixelSize:12 }
                }
                RowLayout {
                    Layout.fillWidth:true; Layout.preferredHeight:24; spacing:12
                    LabelText { Layout.leftMargin:8; Layout.fillWidth:true; text:'Title'; color:Theme.muted; font.pixelSize:12 }
                    LabelText { Layout.preferredWidth:170; text:'Channel'; color:Theme.muted; font.pixelSize:12 }
                    LabelText { Layout.preferredWidth:96; Layout.rightMargin:8; text:'Length'; color:Theme.muted; font.pixelSize:12 }
                }
                Rectangle { Layout.fillWidth:true; implicitHeight:1; color:Theme.line }
                ListView {
                    id:suggestionList; objectName:'youtube-suggestion-list'; Layout.fillWidth:true; Layout.fillHeight:true; clip:true; spacing:0; model:root.suggestionRows; ScrollBar.vertical:SScrollBar {}
                    delegate:AbstractButton {
                        id:suggestionLine; required property var modelData
                        width:suggestionList.width; implicitHeight:26; padding:0; hoverEnabled:true; activeFocusOnTab:true
                        property bool selected:!!root.suggestion && root.suggestion.id === modelData.id
                        Accessible.role:Accessible.ListItem; Accessible.name:(modelData.title||'')+'. '+(modelData.publisher||''); Accessible.selected:selected
                        onClicked:root.suggestion=modelData
                        background:Rectangle {
                            color:suggestionLine.selected ? Theme.wash : suggestionLine.down ? Theme.selection : suggestionLine.hovered ? Theme.hover : 'transparent'
                            border.width:suggestionLine.visualFocus ? 2 : 0; border.color:Theme.accent
                            Rectangle { anchors.bottom:parent.bottom; width:parent.width; height:1; color:Theme.line }
                        }
                        contentItem:RowLayout {
                            spacing:12
                            LabelText { Layout.leftMargin:8; Layout.fillWidth:true; text:Workplane.previewLine(suggestionLine.modelData.title); elide:Text.ElideRight; maximumLineCount:1; font.pixelSize:13 }
                            LabelText { Layout.preferredWidth:170; text:(suggestionLine.modelData.publisher||'')+(suggestionLine.modelData.published?' · '+root.publishedLabel(suggestionLine.modelData.published):''); elide:Text.ElideRight; color:Theme.muted; font.pixelSize:12 }
                            LabelText { Layout.preferredWidth:96; Layout.rightMargin:8; text:suggestionLine.modelData.duration||''; elide:Text.ElideRight; color:Theme.muted; font.pixelSize:12 }
                        }
                    }
                    LabelText { anchors.centerIn:parent; visible:suggestionList.count===0; text:root.query?'No suggestion matches this search.':'No YouTube suggestions on this computer yet.'; color:Theme.muted }
                }
            }
            Rectangle {
                SplitView.fillWidth:true; SplitView.minimumWidth:280; SplitView.minimumHeight:160; color:Theme.base
                ColumnLayout {
                    anchors.fill:parent; anchors.margins:16; spacing:10
                    LabelText { text:root.suggestion?root.suggestion.publisher||'':''; visible:!!text; color:Theme.accent; font.pixelSize:12; Layout.fillWidth:true }
                    LabelText { text:root.suggestion?root.suggestion.title:'Select a suggestion'; font.pixelSize:20; font.weight:Font.DemiBold; Layout.fillWidth:true; wrapMode:Text.Wrap }
                    LabelText { text:root.suggestion?[root.suggestion.duration, root.publishedLabel(root.suggestion.published)].filter(v=>!!v).join(' · '):''; visible:!!text; color:Theme.muted; font.pixelSize:12; Layout.fillWidth:true }
                    LabelText { text:root.suggestion?root.shortNote(root.suggestion.summary, 280):''; visible:!!text; color:Theme.text; Layout.fillWidth:true; wrapMode:Text.Wrap }
                    Flow {
                        Layout.fillWidth:true; spacing:8
                        SButton { text:root.busy?'Adding…':'Collect transcript'; busy:root.busy; primary:true; enabled:!!root.suggestion && !root.busy; onClicked:root.ingest(root.suggestion.url) }
                        SButton { text:'Open on YouTube'; enabled:!!root.suggestion; onClicked:Qt.openUrlExternally(root.suggestion.url) }
                    }
                    Item { Layout.fillHeight:true }
                }
            }
        }

        SplitView {
            visible:root.activeView==='To analyze'; Layout.fillWidth:true; Layout.fillHeight:true; spacing:10
            orientation:root.width < 750 ? Qt.Vertical : Qt.Horizontal
            handle:SSplitHandle { horizontal:root.width >= 750 }
            Rectangle {
                SplitView.preferredWidth:Math.max(280,root.width*.42); SplitView.minimumWidth:240; SplitView.preferredHeight:root.height*.42; SplitView.minimumHeight:160; color:Theme.panel; border.color:Theme.line; radius:Theme.cornerRadius; clip:true
                ColumnLayout { anchors.fill:parent; anchors.margins:10; spacing:6
                    SSearchField { Layout.fillWidth:true; scopeName:'collected videos'; text:root.query; onTextEdited:root.query=text }
                    ListView { id:intakeList; Layout.fillWidth:true; Layout.fillHeight:true; clip:true; spacing:0; model:root.intakeRows.filter(function(item){return !root.query||String(item.title||'').toLowerCase().indexOf(root.query.toLowerCase())>=0}); ScrollBar.vertical:SScrollBar {}
                        delegate:Rectangle { id:intakeRow; required property var modelData; width:intakeList.width; implicitHeight:40; color:String(intakeRow.modelData.id||'')===root.selectedJobId?Theme.selection:'transparent'
                            Rectangle { anchors.bottom:parent.bottom; width:parent.width; height:1; color:Theme.line }
                            RowLayout { id:row; anchors.fill:parent; anchors.leftMargin:8; anchors.rightMargin:4; spacing:8
                                ColumnLayout { Layout.fillWidth:true; spacing:0
                                LabelText { text:intakeRow.modelData.title||intakeRow.modelData.url; Layout.fillWidth:true; elide:Text.ElideRight; font.pixelSize:13 }
                                LabelText { text:[intakeRow.modelData.publisher||intakeRow.modelData.provider||'', root.collectionLabel(intakeRow.modelData.collectionState), intakeRow.modelData.error||''].filter(v=>!!v).join(' · '); Layout.fillWidth:true; color:intakeRow.modelData.error?Theme.dangerText:Theme.muted; font.pixelSize: 12; elide:Text.ElideRight }
                                }
                                SButton { visible:(intakeRow.modelData.history_ids||[]).length>0; text:'Read transcript'; quiet:true; Layout.alignment:Qt.AlignRight; onClicked:root.selectJob(intakeRow.modelData) }
                            }
                        }
                        LabelText { anchors.centerIn:parent; width:Math.min(parent.width-24,420); visible:intakeList.count===0; text:'No video collected yet. Add a YouTube link above, or collect one from Suggestions.'; color:Theme.muted; wrapMode:Text.Wrap; horizontalAlignment:Text.AlignHCenter }
                    }
                }
            }
            Rectangle {
                SplitView.fillWidth:true;SplitView.fillHeight:true;SplitView.minimumWidth:280;SplitView.minimumHeight:230;color:Theme.panel;border.color:Theme.line;radius:Theme.cornerRadius;clip:true
                ColumnLayout {anchors.fill:parent;anchors.margins:12;spacing:8
                    LabelText {text:root.intakeSelected && root.resultRecord?root.resultRecord.title:'Select a source to read';font.pixelSize:18;font.weight:Font.DemiBold;Layout.fillWidth:true;wrapMode:Text.Wrap}
                    LabelText {visible:root.intakeSelected && !root.resultRecord;text:root.selectedJobId?'Transcript is not available yet.':'Choose a collected source to read its transcript.';color:Theme.muted;Layout.fillWidth:true;wrapMode:Text.Wrap}
                    RowLayout {visible:root.intakeSelected && !!root.resultRecord;Layout.fillWidth:true
                        Choice {id:intakeGoal;Layout.fillWidth:true;model:['All goals'].concat(Backend.goals.map(item=>item.title))}
                        Loader {active:Backend.prefs.localModel === true;visible:active;sourceComponent:SButton {text:!Backend.modelChosen?'Waiting for your model':root.busy?'Starting…':'Analyze';busy:root.busy;enabled:Backend.modelChosen && !root.busy && ['queued','running','cancel_requested'].indexOf(root.analysisState)<0;onClicked:root.analyze()}}
                    }
                    DocumentReader {visible:root.intakeSelected && !!root.resultRecord;Layout.fillWidth:true;Layout.fillHeight:true;Layout.minimumHeight:180
                        property var document:TranscriptDocument.build(root.resultRecord?root.resultRecord.segments||[]:[])
                        text:document.text;title:root.resultRecord?root.resultRecord.title:'Transcript'
                        positionKey:'YouTube/intake-transcript/'+root.selectedResultId
                    }
                    LabelText {visible:root.intakeSelected && !!text;text:root.analysisLabel(root.analysisState);color:Theme.muted;Layout.fillWidth:true}
                }
            }
        }

        GridLayout {
            visible:root.activeView==='Results'; Layout.fillWidth:true; Layout.fillHeight:true; rowSpacing:10; columnSpacing:10
            columns: root.width < 750 ? 1 : 2
            Rectangle { Layout.preferredWidth:root.width < 750 ? root.width : 280; Layout.preferredHeight:root.width < 750 ? 150 : -1; Layout.fillHeight:root.width >= 750; color:Theme.panel; border.color:Theme.line; radius:Theme.cornerRadius; clip:true
                ListView { id:jobList; anchors.fill:parent; anchors.margins:6; clip:true; spacing:1; model:root.resultRows; ScrollBar.vertical:SScrollBar {}
                    delegate:Rectangle { id:resultRow; required property var modelData; width:jobList.width; implicitHeight:40; color:String(resultRow.modelData.id||'')===root.selectedJobId?Theme.selection:'transparent'; MouseArea { anchors.fill:parent; onClicked:root.selectJob(resultRow.modelData) }
                        Rectangle { anchors.bottom:parent.bottom; width:parent.width; height:1; color:Theme.line }
                        ColumnLayout {
                            id:jobRow; anchors.fill:parent; anchors.leftMargin:8; anchors.rightMargin:8; spacing:0
                            LabelText { text:resultRow.modelData.title || (resultRow.modelData.result&&resultRow.modelData.result.title?resultRow.modelData.result.title:(root.configured.find(source=>source.id===resultRow.modelData.source_id)||{}).location || 'YouTube source'); Layout.fillWidth:true; elide:Text.ElideRight; font.pixelSize:13 }
                            LabelText { text:resultRow.modelData.error || root.coverageLabel(resultRow.modelData.coverage).replace(/^ · /, '') || 'Analysis'; color:resultRow.modelData.error?Theme.dangerText:Theme.muted; font.pixelSize: 12; Layout.fillWidth:true; elide:Text.ElideRight }
                        }
                    }
                }
            }
            Rectangle { Layout.fillWidth:true; Layout.fillHeight:true; color:Theme.panel; border.color:Theme.line; radius:Theme.cornerRadius; clip:true
                ScrollView { id:resultScroll; anchors.fill:parent; anchors.margins:12; clip:true; contentWidth:availableWidth; ColumnLayout { width:resultScroll.availableWidth; spacing:10
                    LabelText { visible:!!root.resultRecord; text:root.resultRecord?root.resultRecord.title:'Select a result'; font.pixelSize:19; font.weight:Font.DemiBold; Layout.fillWidth:true; wrapMode:Text.Wrap }
                    LabelText { visible:!!root.resultRecord; text:root.resultRecord?(root.resultRecord.channel||(root.resultRecord.provider==='youtube'?'YouTube':root.resultRecord.provider||'')):''; color:Theme.muted; Layout.fillWidth:true }
                    RowLayout {
                        visible: !!root.resultRecord
                        STabButton { text:'Analysis'; enabled:!!root.resultBriefing; active:root.resultView==='Analysis'; onClicked:root.resultView='Analysis' }
                        STabButton { text:'Transcript'; active:root.resultView==='Transcript'; onClicked:root.resultView='Transcript' }
                        Item { Layout.fillWidth:true }
                        SButton { text:'Source'; quiet:true; enabled:root.resultRecord && !!root.resultRecord.url; onClicked:Qt.openUrlExternally(root.resultRecord.url) }
                    }
                    LabelText { visible:!!text; text:root.analysisLabel(root.analysisState); color:Theme.muted; Layout.fillWidth:true }
                    RowLayout {
                        visible:!!root.resultRecord
                        Choice { id:goal; Layout.fillWidth:true; model:['All goals'].concat(Backend.goals.map(item=>item.title)) }
                        Loader {
                            active: Backend.prefs.localModel === true
                            visible: active
                            sourceComponent: SButton {
                busy: root.busy
 text:!Backend.modelChosen?'Waiting for your model':root.busy?'Starting…':'Analyze'; enabled:Backend.modelChosen && !root.busy && ['queued','running','cancel_requested'].indexOf(root.analysisState)<0; onClicked:root.analyze() }
                        }
                    }
                    LabelText { visible:!!root.resultBriefing && root.resultView==='Analysis'; text:root.resultBriefing?root.resultBriefing.summary:''; Layout.fillWidth:true; wrapMode:Text.Wrap }
                    Repeater {
                        model:root.resultBriefing && root.resultView==='Analysis'?(root.resultBriefing.candidate_actions||[]):[]
                        delegate:Rectangle {
                            id:actionCard; required property var modelData; Layout.fillWidth:true; implicitHeight:action.implicitHeight+14; color:Theme.base; border.color:Theme.line; radius:Theme.cornerRadius
                            ColumnLayout {
                                id:action; anchors.fill:parent; anchors.margins:8
                                LabelText { text:actionCard.modelData.title||''; font.weight:Font.DemiBold; Layout.fillWidth:true; wrapMode:Text.Wrap }
                                LabelText { text:actionCard.modelData.rationale||''; color:Theme.muted; Layout.fillWidth:true; wrapMode:Text.Wrap }
                                LabelText { text:(n => n===1?'Cites 1 passage':'Cites '+n+' passages')((actionCard.modelData.segment_ids||[]).length); color:Theme.accent; font.pixelSize: 12 }
                                SButton { text:'Read citations'; quiet:true; onClicked:{root.resultView='Transcript';root.citationIds=actionCard.modelData.segment_ids||[];Qt.callLater(()=>{var index=(root.resultRecord.segments||[]).findIndex(segment=>root.citationIds.indexOf(segment.id)>=0);if(index>=0)transcriptList.positionViewAtIndex(index,ListView.Beginning);});} }
                            }
                        }
                    }
                    DocumentReader {
                        id: transcriptList
                        visible: !!root.resultRecord && root.resultView === 'Transcript'
                        Layout.fillWidth: true; Layout.preferredHeight: Math.max(320, root.height - 220)
                        property var document: TranscriptDocument.build(root.resultRecord ? root.resultRecord.segments || [] : [])
                        property var target: null
                        text: document.text
                        title: root.resultRecord ? root.resultRecord.title : 'Transcript'
                        positionKey: 'YouTube/transcript/' + root.selectedResultId
                        onPassageChanged: (start, end) => target = TranscriptDocument.target(document, start, end)
                        function positionViewAtIndex(index, mode) {
                            var span = document.spans.find(s => s.index === index);
                            if (!span) return;
                            editor.select(span.start, span.end);
                            var rect = editor.positionToRectangle(span.start);
                            scroll.contentY = Math.max(0, Math.min(editor.y + rect.y - 24, scroll.contentHeight - scroll.height));
                        }
                        actions: [
                            LabelText { text: transcriptList.target ? 'Passage ' + (transcriptList.target.index + 1) : 'Transcript'; Layout.maximumWidth: 150; color: Theme.muted; font.pixelSize: 12 },
                            SButton { text: 'Propose work'; quiet: true; enabled: !!transcriptList.target; onClicked: Backend.beginProposal(root.resultRecord, root.resultRecord.segments[transcriptList.target.index]) }
                        ]
                    }
                    LabelText { visible:!root.resultRecord; text:root.selectedJobId?'Transcript is not available yet.':root.resultRows.length?'Select a result to read it.':'No analysis results yet.'; color:Theme.muted; Layout.fillWidth:true; wrapMode:Text.Wrap }
                } }
            }
        }
    }
}
